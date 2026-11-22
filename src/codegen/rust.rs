//! Rust backend.
//!
//! # Artifacts
//!
//! The output directory is one Rust module (`<dir>/mod.rs`):
//!
//! - **types** (`types.rs`): one named type per struct/enum shape in the
//!   [`TypeTable`]. Vandorf enums become Rust enums (void payloads become
//!   unit variants). Implementations use these via `crate::<module>::types`.
//!
//! Vandorf values are immutable, so the expensive ones are shared, and a
//! clone is O(1): arrays are `types::List<T>` (derefs to `[T]`), maps
//! `types::Map<K, V>` (derefs to `BTreeMap`), strings `types::Str`
//! (`Arc<str>`), and fields that make a type contain itself `Arc<T>`. These
//! are defined in `types.rs` itself. `+` and `-` change a value in place
//! when nothing else shares it and copy it otherwise, so sharing is never
//! observable. To keep values unshared, a node's input is moved out of its
//! slot, and a transition moves each value at its last read, taking a
//! node's slot when no later state reads it before the node runs again
//! (see `Moves`). A loop like `acc + [x]` back to the same node appends in
//! place.
//! - **code** (`mod.rs`, `procs.rs`): `pub fn run_<proc>(..)` per proc, plus
//!   private input structs, stubs and helpers.
//!
//! # Model
//!
//! Same state machine as the Go backend: a per-proc `Pc` enum and an env of
//! `Option` slots (reading a node that hasn't run is an error, not a zero
//! value). A `for each` region runs its elements on scoped threads, each with
//! a clone of the env.
//!
//! # Implementations
//!
//! A func's `"file.rs:function"` reference names the function to call:
//!
//! ```rust,ignore
//! pub fn function(<params in declared order, by value>) -> Result<Ret, E>
//! ```
//!
//! where void is `()` and `E: Into<Box<dyn Error + Send + Sync>>`. The file
//! is relative to the output module's directory and maps to a module path the
//! usual way (`src/impls.rs` -> `crate::impls`, `src/a/mod.rs` -> `crate::a`).
//! Funcs without a `.rs` reference call a generated stub that returns an error.
//!
//! # Main
//!
//! With [`RustOptions::main`], `src/main.rs` runs proc `Main` through the
//! crate's library (`<crate>::<module>::run_main`), so the module must be
//! reachable from `lib.rs`. Parameters are parsed from the command line in
//! declared order and the result is printed.

use super::types::{Shape, TyRef, TypeTable};
use super::{Artifact, ImplUse, MainSpec, Output, Writer, exported, gname, join_path, live_after, quote, read_nodes, snake};
use crate::sema::{Arm, BinOp, Body, Branch, Call, Edge, Expr, NodeId, NodeKind, Pat, PatKind, Program, Query, Ref, RegionId, Root, Test, UnOp};
use crate::types::{Prim, Type};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

#[derive(Debug, Clone, Default)]
pub struct RustOptions {
    /// The output directory relative to the crate's `src/` (e.g. `app`), used
    /// to turn impl file paths into module paths.
    pub module_dir: Option<String>,
    /// Executables to generate, each a file under the crate's `src/`.
    pub mains: Vec<MainSpec>,
    /// The library crate's name (as used in paths: `-` becomes `_`), needed
    /// by the main artifact.
    pub crate_name: Option<String>,
    /// HTTP servers to generate: each a file under the crate's `src/`, and
    /// together `http.rs` in the module.
    pub servers: Vec<ServerSpec>,
}

/// An HTTP server whose routes run top-level procs.
#[derive(Debug, Clone, Default)]
pub struct ServerSpec {
    pub name: String,
    pub addr: String,
    /// A top-level proc run before serving, whose result every route and
    /// raw proc takes as `ctx`.
    pub setup: Option<String>,
    /// (method, path, proc), e.g. ("POST", "/api/calc", "Calc"): the proc
    /// takes its parameters and returns its result as JSON.
    pub routes: Vec<(String, String, String)>,
    /// Routes whose proc takes the whole request and returns the whole
    /// response (see RAW_REQUEST and RAW_RESPONSE).
    pub raw: Vec<(String, String, String)>,
    /// (URL path, file as the server's source file reaches it, content
    /// type): embedded with include_str!.
    pub statics: Vec<(String, String, String)>,
    /// The file, relative to the crate's `src/` (`bin/api.rs`).
    pub path: String,
}

pub fn generate(prog: &Program, opts: &RustOptions) -> Result<Output, String> {
    let table = TypeTable::build(prog);
    let module_dir = opts.module_dir.as_deref().map(|d| d.trim_matches('/').to_string());
    let module_path = match &module_dir {
        Some(d) => Some(module_path(d).ok_or_else(|| format!("module dir {d} is not a valid module path"))?),
        None => None,
    };
    let mut g = Gen {
        prog,
        table: &table,
        module_dir,
        coercions: HashMap::new(),
        helpers: vec![],
        stubs: BTreeSet::new(),
        failed: None,
        proc_name: String::new(),
        moves: Default::default(),
        mounts: BTreeMap::new(),
    };
    g.check_names()?;

    let types = Artifact { dir: String::new(), files: BTreeMap::from([("types.rs".to_string(), g.typegen())]) };

    let mut body = Writer::new("    ");
    for &p in &prog.all_procs() {
        g.emit_proc(&mut body, p)?;
    }
    let mut w = Writer::new("    ");
    w.line("// Code generated by vandorf. DO NOT EDIT.");
    w.line("#![allow(unused, non_snake_case, non_camel_case_types, unreachable_patterns, unreachable_code, clippy::all)]");
    w.line("");
    w.line("use super::types;");
    w.raw(RUNTIME);
    w.raw(&body.out);
    for (id, n) in prog.nodes.iter().enumerate() {
        if let NodeKind::Func(_) | NodeKind::Id(_) = &n.kind {
            w.line(&format!("/// Input of {}.", n.qual_name()));
            w.line("#[derive(Debug, Clone)]");
            w.line(&format!("struct {} {{", in_type(prog, id)));
            for (name, t) in prog.params(id) {
                w.line(&format!("{}: {},", rust_ident(name), g.rust_type(t)));
            }
            w.line("}");
            w.line("");
        }
    }
    for &n in &g.stubs.clone() {
        let q = prog.node(n).qual_name();
        w.line(&format!("/// Stands in for {q}, which names no .rs implementation. Write"));
        w.line(&format!("/// `{}`", g.signature(n, "name", "types")));
        w.line("/// and add `= \"file.rs:name\"` to its declaration.");
        let params: Vec<String> = prog.params(n).iter().map(|(_, t)| format!("_: {}", g.rust_type(t))).collect();
        w.line(&format!(
            "fn unimplemented_{}({}) -> Result<{}, Error> {{",
            snake(&gname(prog, n)), params.join(", "), g.rust_type(prog.ret(n)),
        ));
        w.line(&format!("Err({}.into())", quote(&format!("{q} is not implemented"))));
        w.line("}");
        w.line("");
    }
    for h in &g.helpers {
        w.raw(h);
        w.line("");
    }

    let mut mod_rs = "\
// Code generated by vandorf. DO NOT EDIT.
//! Procs generated by vandorf. Implementations take and return the types in
//! [`types`].

pub mod types;
mod procs;

pub use procs::*;
".to_string();
    let mut code = Artifact { dir: String::new(), files: BTreeMap::from([("procs.rs".to_string(), w.out)]) };
    for (name, path) in &g.mounts {
        mod_rs.push_str(&format!("#[path = {}]\nmod {};\n", quote(path), rust_ident(name)));
    }
    if !opts.servers.is_empty() {
        mod_rs.push_str("pub mod http;\n");
        code.files.insert("http.rs".into(), g.http(&opts.servers)?);
    }
    code.files.insert("mod.rs".into(), mod_rs);

    let mains: Vec<Artifact> = opts
        .mains
        .iter()
        .map(|m| main_artifact(prog, &g, opts, m, g.module_dir.as_deref(), module_path.as_deref()))
        .collect::<Result<_, _>>()?;

    let mut mains = mains;
    for srv in &opts.servers {
        mains.push(server_artifact(opts, srv, g.module_dir.as_deref(), module_path.as_deref())?);
    }
    let types_path = module_path.map_or("types".to_string(), |m| format!("{m}::types"));
    if let Some(e) = g.failed.take() {
        return Err(e);
    }
    let impls = prog
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(id, n)| match &n.kind {
            NodeKind::Func(f) => f.imp("rs").map(|i| ImplUse {
                id,
                node: n.qual_name(),
                file: i.file.clone(),
                function: i.function.clone(),
                signature: g.signature(id, &i.function, &types_path),
            }),
            NodeKind::Proc(_) | NodeKind::Alias(_) | NodeKind::Id(_) => None,
        })
        .collect();
    Ok(Output { types, code, mains, impls })
}

fn main_artifact(
    prog: &Program,
    g: &Gen,
    opts: &RustOptions,
    m: &MainSpec,
    module_dir: Option<&str>,
    module_path: Option<&str>,
) -> Result<Artifact, String> {
    let (pid, params) = super::main_proc(prog, m)?;
    let (Some(dir), Some(path)) = (module_dir, module_path) else {
        return Err(format!("{} goes in the crate's src/, so the output's place under it must be known: run inside a crate or pass --module-dir", m.path));
    };
    let krate = opts
        .crate_name
        .as_deref()
        .ok_or_else(|| format!("{} calls the procs through the library crate, whose name is unknown: run inside a crate with a src/lib.rs", m.path))?;
    let run = format!("{}::run_{}", path.replacen("crate", krate, 1), snake(&gname(prog, pid)));
    let usage = super::main_usage(&m.name, &params);

    let mut w = Writer::new("    ");
    w.line("// Code generated by vandorf. DO NOT EDIT.");
    w.line(&format!("//! Runs proc {}, taking its parameters as arguments:", m.entry));
    w.line("//!");
    w.line(&format!("//! ```text\n//! {usage}\n//! ```"));
    w.line("");
    w.line("use std::process::ExitCode;");
    w.line("");
    if matches!(params, super::MainParams::Typed(_)) {
        w.line(&format!("const USAGE: &str = {};", quote(&usage)));
        w.line("");
    }
    w.line("fn main() -> ExitCode {");
    let types = format!("{}::types", path.replacen("crate", krate, 1));
    let mut vars = vec![];
    let typed = match &params {
        super::MainParams::Argv(name) => {
            let var = local(&format!("arg_{name}"));
            w.line(&format!("let {var}: {types}::List<{types}::Str> = std::env::args().skip(1).map(|a| {types}::Str::from(a.as_str())).collect();"));
            vars.push(var);
            &[][..]
        }
        super::MainParams::Typed(ps) => {
            w.line("let args: Vec<String> = std::env::args().skip(1).collect();");
            w.line(&format!("if args.len() != {} {{", ps.len()));
            w.line("eprintln!(\"{USAGE}\");");
            w.line("return ExitCode::from(2);");
            w.line("}");
            &ps[..]
        }
    };
    for (i, (name, p)) in typed.iter().enumerate() {
        let var = local(&format!("arg_{name}"));
        if *p == Prim::String {
            w.line(&format!("let {var} = {types}::Str::from(args[{i}].as_str());"));
        } else {
            w.line(&format!("let {var} = match args[{i}].parse::<{}>() {{", g.rust_type(&Type::Prim(*p))));
            w.line("Ok(v) => v,");
            w.line("Err(e) => {");
            w.line(&format!("eprintln!(\"{name}: {{e}}\\n{{USAGE}}\");"));
            w.line("return ExitCode::from(2);");
            w.line("}");
            w.line("};");
        }
        vars.push(var);
    }
    w.line(&format!("match {run}({}) {{", vars.join(", ")));
    let ret = prog.ret(pid);
    match ret.unfold(&prog.types) {
        Type::Prim(Prim::Void) => w.line("Ok(()) => ExitCode::SUCCESS,"),
        _ => {
            w.line("Ok(result) => {");
            w.line(if matches!(ret.unfold(&prog.types), Type::Prim(_)) { "println!(\"{result}\");" } else { "println!(\"{result:?}\");" });
            w.line("ExitCode::SUCCESS");
            w.line("}");
        }
    }
    w.line("Err(e) => {");
    w.line(&format!("eprintln!(\"error: {{}}\", {}::describe_error(&e));", run.rsplit_once("::").unwrap().0));
    w.line("ExitCode::FAILURE");
    w.line("}");
    w.line("}");
    w.line("}");

    // src/ is `dir` levels above the module.
    let up = vec![".."; dir.split('/').filter(|s| !s.is_empty()).count()];
    let (sub, file) = m.path.trim_matches('/').rsplit_once('/').unwrap_or(("", m.path.as_str()));
    let dir = up.into_iter().chain((!sub.is_empty()).then_some(sub)).collect::<Vec<_>>().join("/");
    Ok(Artifact { dir, files: BTreeMap::from([(file.to_string(), w.out)]) })
}

/// What a raw route's proc takes and returns.
pub const RAW_REQUEST: &[(&str, &str)] =
    &[("method", "string"), ("path", "string"), ("query", "map[string]string"), ("headers", "map[string]string"), ("body", "string")];
pub const RAW_RESPONSE: &[(&str, &str)] = &[("status", "int"), ("headers", "map[string]string"), ("body", "string")];

pub(crate) fn show_shape(fs: &[(&str, &str)]) -> String {
    format!("{{ {} }}", fs.iter().map(|(n, t)| format!("{n}: {t}")).collect::<Vec<_>>().join(", "))
}

/// Whether `t` is a struct of exactly these fields (a raw request or
/// response).
pub(crate) fn raw_shape(prog: &Program, t: &Type, want: &[(&str, &str)]) -> bool {
    let defs = &prog.types;
    matches!(t.unfold(defs), Type::Struct(fs) if fs.len() == want.len()
        && want.iter().all(|(n, w)| fs.iter().any(|(f, ft)| f == n && ft.to_string() == *w || f == n && ft.unfold(defs).to_string() == *w)))
}

/// A server's name as part of a Rust identifier: `web-api` -> `web_api`.
fn server_ident(name: &str) -> String {
    snake(&exported(name)).chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// A server's executable: serves its routes and static files.
fn server_artifact(opts: &RustOptions, srv: &ServerSpec, module_dir: Option<&str>, module_path: Option<&str>) -> Result<Artifact, String> {
    let (Some(dir), Some(path)) = (module_dir, module_path) else {
        return Err(format!("{} goes in the crate's src/, so the output's place under it must be known: run inside a crate or pass --module-dir", srv.path));
    };
    let krate = opts
        .crate_name
        .as_deref()
        .ok_or_else(|| format!("{} serves the procs through the library crate, whose name is unknown: run inside a crate with a src/lib.rs", srv.path))?;
    let http = format!("{}::http", path.replacen("crate", krate, 1));
    let mut w = Writer::new("    ");
    w.line("// Code generated by vandorf. DO NOT EDIT.");
    w.line(&format!("//! HTTP server {} on {} (or --addr host:port):", srv.name, srv.addr));
    w.line("//!");
    for (m, p, proc) in &srv.routes {
        w.line(&format!("//! - `{m} {p}` runs proc {proc}"));
    }
    for (m, p, proc) in &srv.raw {
        w.line(&format!("//! - `{m} {p}` runs proc {proc} on the raw request"));
    }
    for (p, f, _) in &srv.statics {
        w.line(&format!("//! - `GET {p}` serves {f}"));
    }
    if let Some(setup) = &srv.setup {
        w.line("//!");
        w.line(&format!("//! Proc {setup} runs first; its result is each route's ctx."));
    }
    w.line("");
    w.line(&format!("static STATICS: &[{http}::Static] = &["));
    for (p, f, ctype) in &srv.statics {
        w.line(&format!("({}, {}, include_str!({})),", quote(p), quote(ctype), quote(f)));
    }
    w.line("];");
    w.line("");
    w.line("fn main() {");
    let ident = server_ident(&srv.name);
    match srv.setup {
        Some(_) => w.line(&format!("{http}::run_setup({}, {http}::setup_{ident}, {http}::handle_{ident}, STATICS);", quote(&srv.addr))),
        None => w.line(&format!("{http}::run({}, {http}::handle_{ident}, STATICS);", quote(&srv.addr))),
    }
    w.line("}");
    let up = vec![".."; dir.split('/').filter(|s| !s.is_empty()).count()];
    let (sub, file) = srv.path.trim_matches('/').rsplit_once('/').unwrap_or(("", srv.path.as_str()));
    let dir = up.into_iter().chain((!sub.is_empty()).then_some(sub)).collect::<Vec<_>>().join("/");
    Ok(Artifact { dir, files: BTreeMap::from([(file.to_string(), w.out)]) })
}

struct Gen<'a> {
    prog: &'a Program,
    table: &'a TypeTable,
    module_dir: Option<String>,
    coercions: HashMap<(String, String), String>,
    helpers: Vec<String>,
    stubs: BTreeSet<NodeId>,
    /// The first error found while writing an expression.
    failed: Option<String>,
    /// The proc being written, for error messages.
    proc_name: String,
    /// Which reads in the transition being written move their value.
    moves: std::cell::RefCell<Moves>,
    /// Implementation files outside the crate's src/, mounted in mod.rs:
    /// module name -> path relative to the module.
    mounts: BTreeMap<String, String>,
}

/// What a read reads from: `this`, a node's slot in the env, or a switch
/// binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Owner {
    This,
    Node(NodeId),
    Bind(String),
}

/// Last-use moves. A transition's input is written twice: first recording
/// each read's owner and field path in order, then moving every read that
/// no later read overlaps (same owner, one path a prefix of the other). A
/// node's slot is moved from only if no later state reads it before the
/// node runs again; it is then taken into a local (`slot_<node>`) first,
/// so its fields can move separately. Values nothing else shares are then
/// changed in place by `+` and `-`.
#[derive(Debug, Default)]
struct Moves {
    active: bool,
    recording: bool,
    seen: Vec<(Owner, Vec<String>)>,
    moved: HashSet<usize>,
    /// Nodes whose slot this transition takes.
    taken: HashSet<NodeId>,
    next: usize,
}

fn in_type(prog: &Program, n: NodeId) -> String {
    match &prog.node(n).kind {
        NodeKind::Func(_) | NodeKind::Id(_) => format!("In{}", gname(prog, n)),
        NodeKind::Proc(p) => in_type(prog, p.entry),
        NodeKind::Alias(b) => in_type(prog, *b),
    }
}

impl Gen<'_> {
    // ---- Names ----

    fn check_names(&self) -> Result<(), String> {
        let mut seen: HashMap<String, String> = HashMap::new();
        for (id, n) in self.prog.nodes.iter().enumerate() {
            let s = snake(&gname(self.prog, id));
            if let Some(prev) = seen.insert(s.clone(), n.qual_name()) {
                return Err(format!("{prev} and {} both map to the Rust name {s}", n.qual_name()));
            }
        }
        Ok(())
    }

    fn signature(&self, n: NodeId, name: &str, types_path: &str) -> String {
        let q = format!("{types_path}::");
        let params: Vec<String> = self.prog.params(n).iter()
            .map(|(p, t)| format!("{}: {}", local(p), self.render(&self.table.get(t), &q)))
            .collect();
        let ret = self.render(&self.table.get(self.prog.ret(n)), &q);
        format!("pub fn {name}({}) -> Result<{ret}, E>", params.join(", "))
    }

    /// The Rust function a func node calls.
    fn callee(&mut self, n: NodeId) -> Result<String, String> {
        let n = self.prog.base(n);
        let NodeKind::Func(f) = &self.prog.node(n).kind else { unreachable!() };
        let Some(imp) = f.imp("rs") else {
            self.stubs.insert(n);
            return Ok(format!("unimplemented_{}", snake(&gname(self.prog, n))));
        };
        let qual = self.prog.node(n).qual_name();
        let Some(dir) = &self.module_dir else {
            return Err(format!(
                "{qual} is implemented in {}, but the output module's place in the crate is unknown (pass --module-dir or generate under a crate's src/)",
                imp.file,
            ));
        };
        let Some(file) = join_path(dir, &imp.file) else {
            // Outside the crate's src/: mounted in mod.rs with #[path], by
            // its file's stem.
            let stem = imp.file.rsplit('/').next().unwrap_or("").strip_suffix(".rs").unwrap_or("");
            let valid = !stem.is_empty() && stem.chars().enumerate().all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()));
            if !valid || ["types", "procs", "http"].contains(&stem) {
                return Err(format!("{qual}: {} is outside the crate's src/, and its name can't be a module of the output", imp.file));
            }
            if let Some(prev) = self.mounts.get(stem).filter(|p| **p != imp.file) {
                return Err(format!("{qual}: {} and {prev} are both outside the crate's src/ and would both be module {stem}", imp.file));
            }
            self.mounts.insert(stem.to_string(), imp.file.clone());
            return Ok(format!("super::{}::{}", rust_ident(stem), rust_ident(&imp.function)));
        };
        let module = module_path(&file).ok_or_else(|| format!("{qual}: {} is not a Rust module file", imp.file))?;
        Ok(format!("{module}::{}", rust_ident(&imp.function)))
    }

    // ---- HTTP ----

    /// http.rs: the server runtime, Json for every type, and a handler per
    /// server routing to the procs.
    fn http(&self, servers: &[ServerSpec]) -> Result<String, String> {
        let prog = self.prog;
        let mut w = Writer::new("    ");
        w.line("// Code generated by vandorf. DO NOT EDIT.");
        w.line("//! HTTP servers for the procs: requests and results as JSON.");
        w.line("#![allow(unused, non_snake_case, clippy::all)]");
        w.line("");
        w.line("use super::types;");
        w.raw(HTTP_RUNTIME);
        w.line("");
        for t in &self.table.types {
            let name = format!("types::{}", t.name);
            w.line(&format!("impl Json for {name} {{"));
            match &t.shape {
                Shape::Struct(fs) => {
                    w.line("fn to_json(&self, out: &mut String) {");
                    w.line("out.push('{');");
                    for (i, (n, _)) in fs.iter().enumerate() {
                        let key = quote(&format!("{}{}:", if i > 0 { "," } else { "" }, quote(n)));
                        w.line(&format!("out.push_str({key});"));
                        w.line(&format!("self.{}.to_json(out);", rust_ident(n)));
                    }
                    w.line("out.push('}');");
                    w.line("}");
                    w.line("fn from_json(v: &Value) -> Result<Self, String> {");
                    w.line(&format!("let fs = v.object({})?;", quote(&t.name)));
                    w.line(&format!("Ok({name} {{"));
                    for (n, _) in fs {
                        w.line(&format!("{}: Json::from_json(field(fs, {})?).map_err(|e| format!(\"{}: {{e}}\"))?,", rust_ident(n), quote(n), n));
                    }
                    w.line("})");
                    w.line("}");
                }
                Shape::Enum(vs) => {
                    w.line("fn to_json(&self, out: &mut String) {");
                    w.line("match self {");
                    for (n, r) in vs {
                        let v = rust_ident(n);
                        if *r == TyRef::Prim(Prim::Void) {
                            w.line(&format!("{name}::{v} => out.push_str({}),", quote(&quote(n))));
                        } else {
                            w.line(&format!("{name}::{v}(p) => {{"));
                            w.line(&format!("out.push_str({});", quote(&format!("{{{}:", quote(n)))));
                            w.line("p.to_json(out);");
                            w.line("out.push('}');");
                            w.line("}");
                        }
                    }
                    w.line("}");
                    w.line("}");
                    w.line("fn from_json(v: &Value) -> Result<Self, String> {");
                    w.line("match v {");
                    w.line("Value::Str(tag) => match tag.as_str() {");
                    for (n, r) in vs {
                        if *r == TyRef::Prim(Prim::Void) {
                            w.line(&format!("{} => Ok({name}::{}),", quote(n), rust_ident(n)));
                        }
                    }
                    w.line(&format!("t => Err(format!(\"{} has no variant {{t}} without a payload\")),", t.name));
                    w.line("},");
                    w.line("Value::Obj(fs) if fs.len() == 1 => match fs[0].0.as_str() {");
                    for (n, r) in vs {
                        if *r != TyRef::Prim(Prim::Void) {
                            w.line(&format!("{} => Ok({name}::{}(Json::from_json(&fs[0].1)?)),", quote(n), rust_ident(n)));
                        }
                    }
                    w.line(&format!("t => Err(format!(\"{} has no variant {{t}} with a payload\")),", t.name));
                    w.line("},");
                    w.line(&format!("_ => Err({}.into()),", quote(&format!("expected a variant of {}: \"Tag\" or {{\"Tag\": payload}}", t.name))));
                    w.line("}");
                    w.line("}");
                }
            }
            w.line("}");
            w.line("");
        }
        // One call per proc a route runs (servers with a setup get their own,
        // below).
        let plain: Vec<&ServerSpec> = servers.iter().filter(|s| s.setup.is_none()).collect();
        let mut procs: Vec<&str> = plain.iter().flat_map(|s| s.routes.iter().map(|(_, _, p)| p.as_str())).collect();
        procs.sort();
        procs.dedup();
        for p in procs {
            let pid = super::find_proc(prog, p).ok_or_else(|| {
                let srv = plain.iter().find(|s| s.routes.iter().any(|r| r.2 == p)).unwrap();
                format!("http server {} runs proc {p}, but there is no top-level proc {p}", srv.name)
            })?;
            w.line(&format!("/// Runs proc {p} with the request's parameters."));
            w.line(&format!("fn call_{}(params: &Value) -> Result<String, (u16, String)> {{", snake(&gname(prog, pid))));
            let mut args = vec![];
            for (n, t) in prog.params(pid) {
                let var = local(n);
                w.line(&format!("let {var}: {} = arg(params, {})?;", self.rust_type(t), quote(n)));
                args.push(var);
            }
            w.line(&format!("let result = super::run_{}({}).map_err(|e| (500, e.to_string()))?;", snake(&gname(prog, pid)), args.join(", ")));
            w.line("Ok(json(&result))");
            w.line("}");
            w.line("");
        }
        // One raw call per proc a raw route runs.
        let mut raws: Vec<&str> = plain.iter().flat_map(|s| s.raw.iter().map(|(_, _, p)| p.as_str())).collect();
        raws.sort();
        raws.dedup();
        for p in raws {
            let srv = plain.iter().find(|s| s.raw.iter().any(|r| r.2 == p)).unwrap();
            let pid = super::find_proc(prog, p)
                .ok_or_else(|| format!("http server {} runs proc {p}, but there is no top-level proc {p}", srv.name))?;
            let params = prog.params(pid);
            let ok = params.len() == 1 && raw_shape(prog, &params[0].1, RAW_REQUEST) && raw_shape(prog, prog.ret(pid), RAW_RESPONSE);
            if !ok {
                return Err(format!(
                    "http server {}: raw route proc {p} must take one {} and return {}",
                    srv.name, show_shape(RAW_REQUEST), show_shape(RAW_RESPONSE),
                ));
            }
            let fname = snake(&gname(prog, pid));
            w.line(&format!("/// Runs proc {p} on the raw request."));
            w.line(&format!("fn raw_{fname}(req: &Request) -> Response {{"));
            self.raw_body(&mut w, pid, &params[0].1, None);
        }
        for srv in servers {
            if let Some(setup) = &srv.setup {
                self.setup_calls(&mut w, srv, setup)?;
            }
        }
        for srv in servers {
            w.line(&format!("/// The routes of http server {}.", srv.name));
            w.line(&format!("pub fn handle_{}(req: &Request) -> Option<Response> {{", server_ident(&srv.name)));
            w.line("match (req.method.as_str(), req.path.as_str()) {");
            // A server with a setup has calls of its own: call_<server>_<proc>.
            let own = match srv.setup {
                Some(_) => format!("{}_", server_ident(&srv.name)),
                None => String::new(),
            };
            for (m, path, p) in &srv.routes {
                let pid = super::find_proc(prog, p).unwrap();
                w.line(&format!("({}, {}) => Some(json_route(req, call_{own}{})),", quote(m), quote(path), snake(&gname(prog, pid))));
            }
            for (m, path, p) in &srv.raw {
                let pid = super::find_proc(prog, p).unwrap();
                w.line(&format!("({}, {}) => Some(raw_{own}{}(req)),", quote(m), quote(path), snake(&gname(prog, pid))));
            }
            w.line("_ => None,");
            w.line("}");
            w.line("}");
            w.line("");
        }
        Ok(w.out)
    }

    /// The body of a raw call: the request as the proc's request struct
    /// (`req_ty`), and `ctx` from `ctx_static` if the server has a setup.
    fn raw_body(&self, w: &mut Writer, pid: NodeId, req_ty: &Type, ctx: Option<(&str, &str)>) {
        let fname = snake(&gname(self.prog, pid));
        w.line(&format!("let r = {} {{", self.rust_type(req_ty)));
        w.line("method: types::Str::from(req.method.as_str()),");
        w.line("path: types::Str::from(req.path.as_str()),");
        w.line("query: query_pairs(&req.query).into_iter().map(|(k, v)| (types::Str::from(k.as_str()), types::Str::from(v.as_str()))).collect(),");
        w.line("headers: req.headers.iter().map(|(k, v)| (types::Str::from(k.as_str()), types::Str::from(v.as_str()))).collect(),");
        w.line("body: types::Str::from(req.body.as_str()),");
        w.line("};");
        let args = match ctx {
            Some((stat, server)) => {
                w.line(&format!("let ctx = match {stat}.get() {{"));
                w.line("Some(c) => c.clone(),");
                w.line(&format!("None => return error_response(500, {}),", quote(&format!("http server {server} is not set up"))));
                w.line("};");
                self.prog.params(pid).iter().map(|(n, _)| if n == "ctx" { "ctx" } else { "r" }).collect::<Vec<_>>().join(", ")
            }
            None => "r".to_string(),
        };
        w.line(&format!("match super::run_{fname}({args}) {{"));
        w.line("Ok(res) => Response {");
        w.line("status: res.status as u16,");
        w.line("headers: res.headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),");
        w.line("body: res.body.as_bytes().to_vec(),");
        w.line("},");
        w.line("Err(e) => error_response(500, &e.to_string()),");
        w.line("}");
        w.line("}");
        w.line("");
    }

    /// A server with a setup proc: its context, setup_<server> to run the
    /// setup before serving, and a call per route and raw route passing the
    /// context as ctx (never read from the request).
    fn setup_calls(&self, w: &mut Writer, srv: &ServerSpec, setup: &str) -> Result<(), String> {
        let prog = self.prog;
        let sid = super::find_proc(prog, setup)
            .ok_or_else(|| format!("http server {}'s setup runs proc {setup}, but there is no top-level proc {setup}", srv.name))?;
        if !prog.params(sid).is_empty() {
            return Err(format!("http server {}: setup proc {setup} must take no parameters", srv.name));
        }
        let (ctx_ty, ident) = (prog.ret(sid), server_ident(&srv.name));
        let (rctx, stat) = (self.rust_type(ctx_ty), format!("CTX_{}", ident.to_uppercase()));
        // Each of the server's procs must take ctx, of the setup's type.
        let check = |kind: &str, p: &str, pid: NodeId| -> Result<(), String> {
            match prog.params(pid).iter().find(|(n, _)| n == "ctx") {
                None => Err(format!("http server {}: {kind} proc {p} must take ctx: {ctx_ty}, its setup proc {setup}'s result", srv.name)),
                Some((_, t)) if self.rust_type(t) != rctx => {
                    Err(format!("http server {}: {kind} proc {p} takes ctx: {t}, but its setup proc {setup} returns {ctx_ty}", srv.name))
                }
                Some(_) => Ok(()),
            }
        };
        w.line(&format!("/// The context of http server {}: proc {setup}'s result, set before serving.", srv.name));
        w.line(&format!("static {stat}: std::sync::OnceLock<{rctx}> = std::sync::OnceLock::new();"));
        w.line("");
        w.line(&format!("/// Runs proc {setup}, the setup of http server {}.", srv.name));
        w.line(&format!("pub fn setup_{ident}() -> Result<(), String> {{"));
        w.line(&format!("let ctx = super::run_{}().map_err(|e| e.to_string())?;", snake(&gname(prog, sid))));
        w.line(&format!("let _ = {stat}.set(ctx);"));
        w.line("Ok(())");
        w.line("}");
        w.line("");
        let unset = quote(&format!("http server {} is not set up", srv.name));
        let mut procs: Vec<&str> = srv.routes.iter().map(|(_, _, p)| p.as_str()).collect();
        procs.sort();
        procs.dedup();
        for p in procs {
            let pid = super::find_proc(prog, p).ok_or_else(|| format!("http server {} runs proc {p}, but there is no top-level proc {p}", srv.name))?;
            check("route", p, pid)?;
            let fname = snake(&gname(prog, pid));
            w.line(&format!("/// Runs proc {p} with the request's parameters and http server {}'s context.", srv.name));
            w.line(&format!("fn call_{ident}_{fname}(params: &Value) -> Result<String, (u16, String)> {{"));
            let mut args = vec![];
            for (n, t) in prog.params(pid) {
                let var = local(n);
                if n == "ctx" {
                    w.line(&format!("let {var}: {rctx} = {stat}.get().cloned().ok_or_else(|| (500, {unset}.to_string()))?;"));
                } else {
                    w.line(&format!("let {var}: {} = arg(params, {})?;", self.rust_type(t), quote(n)));
                }
                args.push(var);
            }
            w.line(&format!("let result = super::run_{fname}({}).map_err(|e| (500, e.to_string()))?;", args.join(", ")));
            w.line("Ok(json(&result))");
            w.line("}");
            w.line("");
        }
        let mut raws: Vec<&str> = srv.raw.iter().map(|(_, _, p)| p.as_str()).collect();
        raws.sort();
        raws.dedup();
        for p in raws {
            let pid = super::find_proc(prog, p).ok_or_else(|| format!("http server {} runs proc {p}, but there is no top-level proc {p}", srv.name))?;
            check("raw route", p, pid)?;
            let rest: Vec<&(String, Type)> = prog.params(pid).iter().filter(|(n, _)| n != "ctx").collect();
            if !(rest.len() == 1 && raw_shape(prog, &rest[0].1, RAW_REQUEST) && raw_shape(prog, prog.ret(pid), RAW_RESPONSE)) {
                return Err(format!(
                    "http server {}: raw route proc {p} must take ctx and one {}, and return {}",
                    srv.name, show_shape(RAW_REQUEST), show_shape(RAW_RESPONSE),
                ));
            }
            w.line(&format!("/// Runs proc {p} on the raw request with http server {}'s context.", srv.name));
            w.line(&format!("fn raw_{ident}_{}(req: &Request) -> Response {{", snake(&gname(prog, pid))));
            self.raw_body(w, pid, &rest[0].1, Some((&stat, &srv.name)));
        }
        Ok(())
    }

    // ---- Types ----

    fn typegen(&self) -> String {
        let mut w = Writer::new("    ");
        w.line("// Code generated by vandorf. DO NOT EDIT.");
        w.line("#![allow(non_camel_case_types, non_snake_case, dead_code)]");
        w.raw(SHARED);
        for (i, t) in self.table.types.iter().enumerate() {
            if t.origin == format!("typedef {}", t.name) {
                w.line("/// A typedef.");
            } else {
                w.line(&format!("/// The type of the {}.", t.origin));
            }
            w.line("#[derive(Debug, Clone, PartialEq)]");
            match &t.shape {
                Shape::Struct(fs) => {
                    w.line(&format!("pub struct {} {{", t.name));
                    for (n, r) in fs {
                        w.line(&format!("pub {}: {},", rust_ident(n), self.field_type(i, n, r)));
                    }
                }
                Shape::Enum(vs) => {
                    w.line(&format!("pub enum {} {{", t.name));
                    for (n, r) in vs {
                        match r {
                            TyRef::Prim(Prim::Void) => w.line(&format!("{},", rust_ident(n))),
                            r => w.line(&format!("{}({}),", rust_ident(n), self.field_type(i, n, r))),
                        }
                    }
                }
            }
            w.line("}");
            w.line("");
        }
        for (name, r) in &self.table.aliases {
            w.line("/// A typedef.");
            w.line(&format!("pub type {name} = {};", self.render(r, "")));
            w.line("");
        }
        w.out
    }

    /// Field `n` of type `i`, boxed where it would otherwise make the type
    /// contain itself.
    fn field_type(&self, i: usize, n: &str, r: &TyRef) -> String {
        let t = self.render(r, "");
        if self.table.is_boxed(i, n) { format!("std::sync::Arc<{t}>") } else { t }
    }

    fn is_boxed(&self, ty: &Type, part: &str) -> bool {
        self.table.index(ty).is_some_and(|i| self.table.is_boxed(i, part))
    }

    fn render(&self, r: &TyRef, qual: &str) -> String {
        match r {
            TyRef::Prim(p) => match p {
                Prim::Void => "()",
                Prim::Int => "i64",
                Prim::Float => "f64",
                Prim::Bool => "bool",
                Prim::String => return format!("{qual}Str"),
            }
            .into(),
            TyRef::Array(e) => format!("{qual}List<{}>", self.render(e, qual)),
            TyRef::Map(k, v) => format!("{qual}Map<{}, {}>", self.render(k, qual), self.render(v, qual)),
            TyRef::Named(i) => format!("{qual}{}", self.table.types[*i].name),
        }
    }

    /// A vandorf type as written in `procs.rs`.
    fn rust_type(&self, t: &Type) -> String {
        self.render(&self.table.get(t), "types::")
    }

    /// Converts `expr` of type `from` to `to`, which sema checked accepts it.
    fn coerce(&mut self, expr: &str, from: &Type, to: &Type) -> String {
        let (src, dst) = (self.rust_type(from), self.rust_type(to));
        if src == dst {
            return expr.to_string();
        }
        let key = (src.clone(), dst.clone());
        if let Some(name) = self.coercions.get(&key) {
            return format!("{name}({expr})");
        }
        let name = format!("coerce_{}", self.coercions.len());
        self.coercions.insert(key, name.clone());
        let mut w = Writer::new("    ");
        w.line(&format!("fn {name}(v: {src}) -> {dst} {{"));
        let prog = self.prog;
        let defs = &prog.types;
        // Converts part `n` (a field, or variant `n`'s payload) held in
        // `expr`, through boxes where either side has one.
        let part = |g: &mut Self, expr: &str, n: &str, f: &Type, t: &Type| {
            let e = g.coerce(&if g.is_boxed(from, n) { format!("(*{expr}).clone()") } else { expr.to_string() }, f, t);
            if g.is_boxed(to, n) { format!("std::sync::Arc::new({e})") } else { e }
        };
        match (from.unfold(defs), to.unfold(defs)) {
            (Type::Array(f), Type::Array(t)) => {
                let e = self.coerce("x", f, t);
                w.line(&format!("v.into_iter().map(|x| {e}).collect()"));
            }
            (Type::Map(_, f), Type::Map(_, t)) => {
                let e = self.coerce("x", f, t);
                w.line(&format!("v.into_iter().map(|(k, x)| (k, {e})).collect()"));
            }
            (Type::Struct(_), Type::Struct(tfs)) => {
                w.line(&format!("{dst} {{"));
                for (n, t) in tfs {
                    let f = rust_ident(n);
                    let e = part(self, &format!("v.{f}"), n, from.field(n, defs).unwrap(), t);
                    w.line(&format!("{f}: {e},"));
                }
                w.line("}");
            }
            (Type::Enum(fvs), Type::Enum(_)) => {
                w.line("match v {");
                for (n, f) in fvs {
                    let v = rust_ident(n);
                    if f.is_void() {
                        w.line(&format!("{src}::{v} => {dst}::{v},"));
                    } else {
                        let e = part(self, "p", n, f, to.variant(n, defs).unwrap());
                        w.line(&format!("{src}::{v}(p) => {dst}::{v}({e}),"));
                    }
                }
                w.line("}");
            }
            _ => unreachable!("sema admitted coercion {src} -> {dst}"),
        }
        w.line("}");
        self.helpers.push(w.out);
        format!("{name}({expr})")
    }

    // ---- Expressions ----

    fn reference(&self, r: &Ref) -> String {
        let owner = match r.root {
            Root::This => Owner::This,
            Root::Node(n) => Owner::Node(n),
            Root::Bind => Owner::Bind(r.bind.clone().unwrap()),
        };
        // Field access sees through boxes; a boxed last field is unboxed.
        let mut ty = &r.root_ty;
        let mut boxed = false;
        let mut through_box = false;
        let mut path = String::new();
        for f in &r.fields {
            through_box |= boxed;
            boxed = self.is_boxed(ty, f);
            ty = ty.field(f, &self.prog.types).unwrap();
            write!(path, ".{}", rust_ident(f)).unwrap();
        }
        // A value can't be moved out from behind an `Arc`.
        let moving = self.should_move(owner, &r.fields) && !through_box;
        let root = match r.root {
            Root::This => "this".to_string(),
            Root::Node(n) if self.moves.borrow().taken.contains(&n) => format!("slot_{}", snake(&gname(self.prog, n))),
            Root::Node(n) => format!("get(&env.out_{}, {})?", snake(&gname(self.prog, n)), quote(&self.prog.node(n).qual_name())),
            Root::Bind => bind_var(r.bind.as_deref().unwrap()),
        };
        let s = root + &path;
        match (moving, boxed) {
            (true, true) => format!("std::sync::Arc::unwrap_or_clone({s})"),
            (true, false) => s,
            (false, true) => format!("(*{s}).clone()"),
            (false, false) => s + ".clone()",
        }
    }

    /// Records a read of `owner`, and says whether it moves (see [`Moves`]).
    fn should_move(&self, owner: Owner, path: &[String]) -> bool {
        let mut m = self.moves.borrow_mut();
        if !m.active {
            return false;
        }
        let i = m.next;
        m.next += 1;
        if m.recording {
            m.seen.push((owner, path.to_vec()));
            return false;
        }
        m.moved.contains(&i)
    }

    /// `e` as a Rust expression of type `want`. Calls in it use `?`, so it
    /// goes in a function returning `Result<_, Error>`.
    fn value(&mut self, e: &Expr, want: &Type) -> String {
        let prog = self.prog;
        let defs = &prog.types;
        let boxed = |g: &Self, part: &str, v: String| if g.is_boxed(want, part) { format!("std::sync::Arc::new({v})") } else { v };
        match e {
            Expr::Ref(r) => self.coerce(&self.reference(r), &r.ty, want),
            Expr::Call(c) => {
                let v = self.own(e);
                self.coerce(&v, prog.ret(c.target), want)
            }
            Expr::Array(es) => {
                let Type::Array(elem) = want.unfold(defs) else { unreachable!() };
                let items: Vec<String> = es.iter().map(|e| self.value(e, elem)).collect();
                format!("types::List::from(vec![{}])", items.join(", "))
            }
            Expr::Struct(fs) => {
                let items: Vec<String> = fs
                    .iter()
                    .map(|(n, e)| {
                        let v = self.value(e, want.field(n, defs).unwrap());
                        format!("{}: {}", rust_ident(n), boxed(self, n, v))
                    })
                    .collect();
                format!("{} {{ {} }}", self.rust_type(want), items.join(", "))
            }
            Expr::Map(kvs) => {
                let Type::Map(kt, vt) = want.unfold(defs) else { unreachable!() };
                let items: Vec<String> = kvs.iter().map(|(k, v)| format!("({}, {})", self.value(k, kt), self.value(v, vt))).collect();
                format!("types::Map::from_iter([{}])", items.join(", "))
            }
            Expr::Index { ty, .. } => {
                let v = self.own(e);
                let vt = match ty.unfold(defs) {
                    Type::Map(_, v) | Type::Array(v) => v,
                    _ => unreachable!(),
                };
                self.coerce(&v, vt, want)
            }
            Expr::Field { ty, name, .. } => {
                let v = self.own(e);
                self.coerce(&v, ty.field(name, defs).unwrap(), want)
            }
            Expr::Merge { ty, .. } | Expr::Remove { ty, .. } => {
                let v = self.own(e);
                self.coerce(&v, ty, want)
            }
            Expr::Query { ty, q: q @ (Query::Keys | Query::Values), .. } => {
                let v = self.own(e);
                let Type::Map(k, v_) = ty.unfold(defs) else { unreachable!() };
                let item = if *q == Query::Keys { k } else { v_ };
                self.coerce(&v, &Type::Array(item.clone()), want)
            }
            Expr::Variant { tag, payload } => {
                let path = format!("{}::{}", self.rust_type(want), rust_ident(tag));
                match payload {
                    None => path,
                    Some(p) => {
                        let v = self.value(p, want.variant(tag, defs).unwrap());
                        format!("{path}({})", boxed(self, tag, v))
                    }
                }
            }
            // Primitives, whose Rust types match whenever sema accepted them.
            _ => self.own(e),
        }
    }

    /// An expression that has its own type (not a struct, array or variant
    /// literal), at that type.
    fn own(&mut self, e: &Expr) -> String {
        let prog = self.prog;
        match e {
            Expr::Ref(r) => self.reference(r),
            Expr::Int(i) => format!("{i}_i64"),
            Expr::Float(f) => format!("{f:?}_f64"),
            Expr::Str(s) => format!("types::Str::from({})", quote(s)),
            Expr::Bool(b) => b.to_string(),
            Expr::Unary(op, x) => format!("({}{})", if *op == UnOp::Not { "!" } else { "-" }, self.own(x)),
            Expr::Binary { op: BinOp::Add, l, r, operand: crate::types::Prim::String } => {
                format!("types::Str::from(format!(\"{{}}{{}}\", {}, {}))", self.own(l), self.own(r))
            }
            Expr::Binary { op, l, r, .. } => format!("({} {} {})", self.own(l), op.symbol(), self.own(r)),
            Expr::Call(c) => {
                let callee = self.callee(c.target).unwrap_or_else(|e| {
                    // Reported by generate() once the expression is written.
                    self.failed.get_or_insert(e);
                    "unresolved".into()
                });
                let args: Vec<String> = c.args.iter().map(|a| self.value(&a.value, &a.param)).collect();
                format!(
                    "{callee}({}).map_err(|e| wrap({}, e.into()))?",
                    args.join(", "),
                    quote(&prog.node(c.target).qual_name()),
                )
            }
            Expr::Index { e, key, ty } => {
                let get = if matches!(ty.unfold(&prog.types), Type::Map(..)) { "map_get" } else { "slice_get" };
                format!("{get}(&{}, &{}).map_err(|e| wrap({}, e))?", self.own(e), self.own(key), quote(&self.proc_name))
            }
            Expr::Field { e, ty, name } => {
                // The struct may be a literal (`{ ..x, f = v }.g`): render it
                // at its type.
                let v = format!("({}).{}", self.value(e, ty), rust_ident(name));
                if self.is_boxed(ty, name) { format!("(*{v})") } else { v }
            }
            Expr::In { key, map } => format!("({}).contains_key(&{})", self.own(map), self.own(key)),
            Expr::Query { e, q, .. } => {
                let x = self.own(e);
                match q {
                    Query::Len => format!("(({x}).len() as i64)"),
                    Query::Keys => format!("({x}).keys().cloned().collect::<types::List<_>>()"),
                    Query::Values => format!("({x}).values().cloned().collect::<types::List<_>>()"),
                }
            }
            Expr::Merge { l, r, ty } => {
                let (l, r) = (self.value(l, ty), self.value(r, ty));
                let f = if matches!(ty.unfold(&prog.types), Type::Array(_)) { "concat" } else { "merge" };
                format!("({l}).{f}(&{r})")
            }
            Expr::Remove { m, keys, ty } => {
                let Type::Map(k, _) = ty.unfold(&prog.types) else { unreachable!() };
                let (m, ks) = (self.own(m), self.value(keys, &Type::Array(k.clone())));
                format!("({m}).without(&{ks})")
            }
            Expr::Struct(_) | Expr::Array(_) | Expr::Variant { .. } | Expr::Map(_) => unreachable!("literal without a type"),
        }
    }

    fn input(&mut self, c: &Call) -> String {
        let mut parts = vec![];
        for a in &c.args {
            let v = self.value(&a.value, &a.param);
            parts.push(format!("{}: {v}", rust_ident(&a.name)));
        }
        format!("{} {{ {} }}", in_type(self.prog, c.target), parts.join(", "))
    }

    // ---- Procs ----

    fn emit_proc(&mut self, w: &mut Writer, pid: NodeId) -> Result<(), String> {
        let prog = self.prog;
        self.proc_name = prog.node(pid).qual_name();
        let proc = prog.proc(pid);
        let g = &proc.graph;
        let name = gname(prog, pid);
        let env = format!("Env{name}");
        let pc = format!("Pc{name}");
        let order: Vec<NodeId> = g.regions.iter().flat_map(|r| r.nodes.iter().copied()).collect();

        w.line(&format!("// ---- proc {} ----", prog.node(pid).qual_name()));
        w.line("");
        w.line("#[derive(Debug, Clone, Default)]");
        w.line(&format!("struct {env} {{"));
        for &n in &order {
            let id = snake(&gname(prog, n));
            w.line(&format!("in_{id}: Option<{}>,", in_type(prog, n)));
            w.line(&format!("out_{id}: Option<{}>,", self.rust_type(prog.ret(n))));
        }
        w.line("}");
        w.line("");
        w.line("#[derive(Debug, Clone, Copy, PartialEq, Eq)]");
        w.line(&format!("enum {pc} {{"));
        for &n in &order {
            w.line(&format!("{},", gname(prog, n)));
        }
        w.line("}");
        w.line("");

        let params = prog.params(pid);
        let ret = self.rust_type(&proc.ret);
        let sig: Vec<String> = params.iter().map(|(p, t)| format!("{}: {}", local(p), self.rust_type(t))).collect();
        w.line(&format!("/// Runs proc {}.", prog.node(pid).qual_name()));
        w.line(&format!("pub fn run_{}({}) -> Result<{ret}, Error> {{", snake(&name), sig.join(", ")));
        w.line(&format!("let mut env = {env}::default();"));
        let fields: Vec<String> = params.iter().map(|(p, _)| format!("{}: {}", rust_ident(p), local(p))).collect();
        w.line(&format!(
            "env.in_{} = Some({} {{ {} }});",
            snake(&gname(prog, proc.entry)), in_type(prog, proc.entry), fields.join(", "),
        ));
        w.line(&format!("let mut pc = {pc}::{};", gname(prog, proc.entry)));
        self.emit_loop(w, pid, 0)?;
        w.line("}");
        w.line("");

        let mut opens: Vec<(NodeId, RegionId)> = g.opens.iter().map(|(&n, &r)| (n, r)).collect();
        opens.sort_by_key(|&(_, r)| r);
        for (opener, r) in opens {
            let region = &g.regions[r];
            let Some(Edge::ForEach { call, .. }) = g.edges.get(&opener) else { unreachable!() };
            let sink = self.rust_type(prog.ret(region.sink.unwrap()));
            w.line(&format!("/// One element of the for each from {}.", prog.node(opener).qual_name()));
            w.line(&format!(
                "fn {}(mut env: {env}, elem: {}) -> Result<{sink}, Error> {{",
                region_fn(prog, pid, r), self.rust_type(&region.elem),
            ));
            w.line("let this = elem;");
            let input = self.input(call);
            w.line(&format!("env.in_{} = Some({input});", snake(&gname(prog, region.entry))));
            w.line(&format!("let mut pc = {pc}::{};", gname(prog, region.entry)));
            self.emit_loop(w, pid, r)?;
            w.line("}");
            w.line("");
        }
        Ok(())
    }

    fn emit_loop(&mut self, w: &mut Writer, pid: NodeId, r: RegionId) -> Result<(), String> {
        w.line("loop {");
        w.line("match pc {");
        for &n in &self.prog.proc(pid).graph.regions[r].nodes {
            self.emit_node(w, pid, n)?;
        }
        let msg = quote(&format!("{}: invalid state {{pc:?}}", self.prog.node(pid).qual_name()));
        w.line(&format!("_ => return Err(format!({msg}).into()),"));
        w.line("}");
        w.line("}");
        Ok(())
    }

    fn emit_node(&mut self, w: &mut Writer, pid: NodeId, n: NodeId) -> Result<(), String> {
        let prog = self.prog;
        let proc = prog.proc(pid);
        let id = snake(&gname(prog, n));
        let qual = prog.node(n).qual_name();
        let ret = prog.ret(n);
        let base = prog.base(n);
        let callee = match prog.node(base).kind {
            NodeKind::Proc(_) => format!("run_{}", snake(&gname(prog, base))),
            NodeKind::Id(_) => String::new(),
            _ => self.callee(n)?,
        };
        let args: Vec<String> = prog.params(n).iter().map(|(p, _)| format!("input.{}", rust_ident(p))).collect();

        w.line(&format!("Pc{}::{} => {{", gname(prog, pid), gname(prog, n)));
        w.line(&format!("let input = take_slot(&mut env.in_{id}, {})?;", quote(&qual)));
        if let NodeKind::Id(_) = prog.node(n).kind {
            // id: the value is the output.
            w.line("let this = input.value;");
        } else {
            w.line(&format!(
                "let this = {callee}({}).map_err(|e| wrap({}, e.into()))?;",
                args.join(", "), quote(&qual),
            ));
        }
        if read_nodes(&proc.graph).contains(&n) {
            w.line(&format!("env.out_{id} = Some(this.clone());"));
        }

        match proc.graph.edges.get(&n) {
            None => {
                let e = self.coerce("this", ret, &proc.ret);
                w.line(&format!("return Ok({e});"));
            }
            Some(Edge::Call(c)) => self.transition(w, pid, c),
            Some(Edge::Collect(_)) => w.line("return Ok(this);"),
            Some(Edge::Match(arms)) => self.emit_switch(w, pid, n, arms, true),
            Some(Edge::ForEach { dim, .. }) => {
                let r = proc.graph.opens[&n];
                let sink = proc.graph.regions[r].sink.unwrap();
                w.line(&format!("let dim = {};", self.own(dim)));
                w.line(&format!("let results = fan_out(dim.to_vec(), |elem| {}(env.clone(), elem))?;", region_fn(prog, pid, r)));
                w.line("let this = types::List::from(results);");
                let Some(Edge::Collect(c)) = proc.graph.edges.get(&sink) else { unreachable!() };
                self.transition(w, pid, c);
            }
            Some(Edge::Spawn { branches, join, joined }) => {
                self.emit_spawn(w, branches, joined)?;
                self.transition(w, pid, join);
            }
        }
        w.line("}");
        Ok(())
    }

    /// A spawn's branches, each a call of its node: detached ones on threads
    /// of their own, which log a failure; joined ones on scoped threads
    /// this step waits for, failing if any fails. `this` becomes the joined
    /// outputs. The arguments are all built first, from the spawner's step.
    fn emit_spawn(&mut self, w: &mut Writer, branches: &[Branch], joined: &Type) -> Result<(), String> {
        let prog = self.prog;
        let mut calls = vec![];
        for (i, b) in branches.iter().enumerate() {
            let n = b.call.target;
            let mut args = vec![];
            for a in &b.call.args {
                let v = self.value(&a.value, &a.param);
                let var = format!("a{i}_{}", local(&a.name));
                w.line(&format!("let {var} = {v};"));
                args.push(var);
            }
            let base = prog.base(n);
            let callee = match prog.node(base).kind {
                NodeKind::Proc(_) => format!("run_{}", snake(&gname(prog, base))),
                _ => self.callee(n)?,
            };
            calls.push(format!("{callee}({}).map_err(|e| wrap({}, e.into()))", args.join(", "), quote(&prog.node(n).qual_name())));
        }
        for i in (0..branches.len()).filter(|&i| branches[i].detach) {
            w.line(&format!(
                "thread_builder().spawn(move || if let Err(e) = {} {{ eprintln!(\"error: {{}}\", describe_error(&e)); }}).expect(\"cannot spawn a thread\");",
                calls[i],
            ));
        }
        w.line("let this = std::thread::scope(|s| -> Result<_, Error> {");
        let waited: Vec<usize> = (0..branches.len()).filter(|&i| !branches[i].detach).collect();
        for &i in &waited {
            w.line(&format!("let h{i} = thread_builder().spawn_scoped(s, move || {}).expect(\"cannot spawn a thread\");", calls[i]));
        }
        for &i in &waited {
            w.line(&format!("let r{i} = h{i}.join().unwrap_or_else(|p| std::panic::resume_unwind(p))?;"));
        }
        let mut fields = vec![];
        for &i in &waited {
            let name = prog.node(branches[i].call.target).name();
            if joined.field(name, &prog.types).is_some() {
                let v = if self.is_boxed(joined, name) { format!("std::sync::Arc::new(r{i})") } else { format!("r{i}") };
                fields.push(format!("{}: {v}", rust_ident(name)));
            }
        }
        w.line(&format!("Ok({} {{ {} }})", self.rust_type(joined), fields.join(", ")));
        w.line("})?;");
        Ok(())
    }

    /// Arms in order, as an `if` chain: a transition ends with `continue`,
    /// so the first arm that matches is taken. Sema checked that one always
    /// does.
    fn emit_switch(&mut self, w: &mut Writer, pid: NodeId, n: NodeId, arms: &[Arm], top: bool) {
        let qual = self.prog.node(n).qual_name();
        let mut always = false;
        for a in arms {
            match &a.test {
                Test::Else => {
                    always = true;
                    self.emit_body(w, pid, n, &a.body);
                }
                Test::Cond(c) => {
                    w.line(&format!("if {} {{", self.own(c)));
                    self.emit_body(w, pid, n, &a.body);
                    w.line("}");
                }
                Test::Pat(p) => {
                    let test = self.pat_test(p, "(&this)");
                    always = test.is_none();
                    w.line(&match test {
                        Some(t) => format!("if {t} {{"),
                        None => "{".to_string(),
                    });
                    let mut binds = vec![];
                    self.pat_binds(p, "(&this)".into(), &mut binds);
                    for (b, v) in binds {
                        w.line(&format!("let {} = {v}.clone();", bind_var(&b)));
                    }
                    self.emit_body(w, pid, n, &a.body);
                    w.line("}");
                }
            }
        }
        if top && !always {
            w.line(&format!("return Err(format!({}, this).into());", quote(&format!("{qual}: no match arm for {{:?}}"))));
        }
    }

    fn emit_body(&mut self, w: &mut Writer, pid: NodeId, n: NodeId, body: &Body) {
        match body {
            Body::Call(c) => self.transition(w, pid, c),
            Body::Switch(arms) => self.emit_switch(w, pid, n, arms, false),
        }
    }

    /// A bool expression testing whether the value `x` (an expression of
    /// type `&T`) matches `p`, or `None` if it always does. Patterns can't
    /// see through boxes, so nested parts are tested in `matches!` guards.
    fn pat_test(&self, p: &Pat, x: &str) -> Option<String> {
        match &p.kind {
            PatKind::Wild | PatKind::Bind(_) => None,
            PatKind::Variant { tag, payload } => {
                let path = format!("{}::{}", self.rust_type(&p.ty), rust_ident(tag));
                let Some(q) = payload else { return Some(format!("matches!({x}, {path})")) };
                let inner = if self.is_boxed(&p.ty, tag) { "&**p" } else { "p" };
                Some(match self.pat_test(q, &format!("({inner})")) {
                    None => format!("matches!({x}, {path}(..))"),
                    Some(t) => format!("matches!({x}, {path}(p) if {t})"),
                })
            }
            PatKind::Struct(fs) => {
                let tests: Vec<String> = fs.iter().filter_map(|(f, q)| self.pat_test(q, &self.field_ref(&p.ty, x, f))).collect();
                (!tests.is_empty()).then(|| format!("({})", tests.join(" && ")))
            }
            PatKind::Or(alts) => {
                let mut tests = vec![];
                for a in alts {
                    tests.push(self.pat_test(a, x)?);
                }
                Some(format!("({})", tests.join(" || ")))
            }
        }
    }

    /// Expressions (of type `&T`) for the bindings in `p`, which `x` is
    /// known to match.
    fn pat_binds(&self, p: &Pat, x: String, out: &mut Vec<(String, String)>) {
        match &p.kind {
            PatKind::Wild | PatKind::Or(_) => {}
            PatKind::Bind(b) => out.push((b.clone(), x)),
            PatKind::Variant { tag, payload: Some(q) } => {
                let path = format!("{}::{}", self.rust_type(&p.ty), rust_ident(tag));
                let inner = if self.is_boxed(&p.ty, tag) { "&**p" } else { "p" };
                self.pat_binds(q, format!("(match {x} {{ {path}(p) => {inner}, _ => unreachable!() }})"), out);
            }
            PatKind::Variant { payload: None, .. } => {}
            PatKind::Struct(fs) => {
                for (f, q) in fs {
                    self.pat_binds(q, self.field_ref(&p.ty, &x, f), out);
                }
            }
        }
    }

    /// `&x.f`, unboxed.
    fn field_ref(&self, ty: &Type, x: &str, f: &str) -> String {
        if self.is_boxed(ty, f) { format!("(&*{x}.{})", rust_ident(f)) } else { format!("(&{x}.{})", rust_ident(f)) }
    }

    fn transition(&mut self, w: &mut Writer, pid: NodeId, c: &Call) {
        let g = &self.prog.proc(pid).graph;
        // First pass: who is read, in order.
        *self.moves.borrow_mut() = Moves { active: true, recording: true, ..Default::default() };
        self.input(c);
        let seen = std::mem::take(&mut self.moves.borrow_mut().seen);
        let taken: HashSet<NodeId> = seen
            .iter()
            .filter_map(|(o, _)| if let Owner::Node(n) = o { Some(*n) } else { None })
            .filter(|&n| !live_after(g, c.target, n))
            .collect();
        let overlaps = |a: &[String], b: &[String]| a.iter().zip(b).all(|(x, y)| x == y);
        let moved = (0..seen.len())
            .filter(|&i| {
                let (o, p) = &seen[i];
                let movable = match o {
                    Owner::Node(n) => taken.contains(n),
                    Owner::This | Owner::Bind(_) => true,
                };
                movable && !seen[i + 1..].iter().any(|(o2, p2)| o2 == o && overlaps(p, p2))
            })
            .collect();
        let mut taken: Vec<NodeId> = taken.into_iter().collect();
        taken.sort();
        *self.moves.borrow_mut() = Moves { active: true, moved, taken: taken.iter().copied().collect(), ..Default::default() };
        let input = self.input(c);
        *self.moves.borrow_mut() = Moves::default();
        for n in taken {
            let id = snake(&gname(self.prog, n));
            w.line(&format!("let slot_{id} = take_slot(&mut env.out_{id}, {})?;", quote(&self.prog.node(n).qual_name())));
        }
        let target = gname(self.prog, c.target);
        w.line(&format!("env.in_{} = Some({input});", snake(&target)));
        w.line(&format!("pc = Pc{}::{target};", gname(self.prog, pid)));
        w.line("continue;");
    }
}

/// The Rust variable of a switch-arm binding.
fn bind_var(name: &str) -> String {
    format!("b_{name}")
}

fn region_fn(prog: &Program, pid: NodeId, r: RegionId) -> String {
    format!("run_{}_region{r}", snake(&gname(prog, pid)))
}

/// `impls.rs` -> `crate::impls`, `a/mod.rs` -> `crate::a`, `lib.rs` -> `crate`
/// (paths relative to the crate's `src/`). Also accepts a directory.
fn module_path(file: &str) -> Option<String> {
    let stem = file.strip_suffix(".rs").unwrap_or(file);
    let mut segs: Vec<&str> = stem.split('/').filter(|s| !s.is_empty()).collect();
    if segs.last() == Some(&"mod") {
        segs.pop();
    }
    if segs == ["lib"] || segs == ["main"] {
        segs.clear();
    }
    let valid = |s: &str| {
        s.chars().enumerate().all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
    };
    if !segs.iter().all(|s| valid(s)) {
        return None;
    }
    let mut path = String::from("crate");
    for s in segs {
        path.push_str("::");
        path.push_str(&rust_ident(s));
    }
    Some(path)
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "else", "enum", "extern", "false", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static",
    "struct", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn",
    "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized",
    "virtual", "yield", "try", "gen",
];

/// A Rust identifier for a vandorf name: raw for keywords, suffixed for the
/// few that can't be raw.
fn rust_ident(s: &str) -> String {
    if ["self", "Self", "super", "crate"].contains(&s) {
        format!("{s}_")
    } else if RUST_KEYWORDS.contains(&s) {
        format!("r#{s}")
    } else {
        s.to_string()
    }
}

/// A function parameter or local, kept clear of the generated locals.
fn local(s: &str) -> String {
    if ["env", "pc", "this", "input", "results", "elem", "dim"].contains(&s) {
        format!("{s}_")
    } else {
        rust_ident(s)
    }
}

const HTTP_RUNTIME: &str = include_str!("http_runtime.rs");

const RUNTIME: &str = r#"
/// Error returned by procs.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// The node that failed, wrapping its implementation's error.
#[derive(Debug)]
pub struct NodeError {
    pub node: &'static str,
    pub source: Error,
}

impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.node, self.source)
    }
}

impl std::error::Error for NodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

fn wrap(node: &'static str, source: Error) -> Error {
    Box::new(NodeError { node, source })
}

/// A failure as a program prints it: the error underneath the nodes, and
/// (when it passed through more than one) the nodes, outermost first.
pub fn describe_error(e: &Error) -> String {
    let mut nodes = vec![];
    let mut cur: &(dyn std::error::Error + Send + Sync + 'static) = &**e;
    while let Some(n) = cur.downcast_ref::<NodeError>() {
        nodes.push(n.node);
        cur = &*n.source;
    }
    if nodes.len() > 1 { format!("{cur}\n  (in {})", nodes.join(" > ")) } else { cur.to_string() }
}

fn get<'a, T>(slot: &'a Option<T>, node: &str) -> Result<&'a T, Error> {
    slot.as_ref().ok_or_else(|| format!("{node} has not run").into())
}

/// Moves a value out of its slot, at its last read.
fn take_slot<T>(slot: &mut Option<T>, node: &str) -> Result<T, Error> {
    slot.take().ok_or_else(|| format!("{node} has not run").into())
}

/// Looks up `k`, failing if it's missing.
fn map_get<K: Ord + std::fmt::Debug, V: Clone>(m: &std::collections::BTreeMap<K, V>, k: &K) -> Result<V, Error> {
    m.get(k).cloned().ok_or_else(|| format!("key {k:?} not found").into())
}

/// Indexes `xs`, failing if `i` is out of range.
fn slice_get<E: Clone>(xs: &[E], i: &i64) -> Result<E, Error> {
    usize::try_from(*i)
        .ok()
        .and_then(|i| xs.get(i).cloned())
        .ok_or_else(|| format!("index {i} out of range [0, {})", xs.len()).into())
}

/// Unoptimized, a proc's frame holds every node's temporaries at once, so
/// grammar-deep recursion overflows the default 2 MiB; debug builds give
/// their threads more.
fn thread_builder() -> std::thread::Builder {
    let b = std::thread::Builder::new();
    #[cfg(debug_assertions)]
    let b = b.stack_size(64 << 20);
    b
}

/// Runs `f` over `xs` on scoped threads; results keep input order.
fn fan_out<E: Send, R: Send>(xs: Vec<E>, f: impl Fn(E) -> Result<R, Error> + Sync) -> Result<Vec<R>, Error> {
    std::thread::scope(|s| {
        let f = &f;
        let handles: Vec<_> = xs
            .into_iter()
            .map(|x| thread_builder().spawn_scoped(s, move || f(x)).expect("cannot spawn a thread"))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
            .collect()
    })
}

"#;

/// Shared immutable values, at the top of every `types.rs`.
const SHARED: &str = r#"
use std::collections::BTreeMap;
use std::sync::Arc;

/// A string. Immutable, so a clone shares it.
pub type Str = Arc<str>;

/// An array. Immutable, so a clone shares it. `concat` reuses this one's
/// buffer when nothing else shares it, and copies otherwise. Derefs to a
/// slice.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct List<T>(Arc<Vec<T>>);

impl<T> Clone for List<T> {
    fn clone(&self) -> Self {
        List(self.0.clone())
    }
}

impl<T> Default for List<T> {
    fn default() -> Self {
        List(Arc::new(Vec::new()))
    }
}

impl<T> std::ops::Deref for List<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T> From<Vec<T>> for List<T> {
    fn from(v: Vec<T>) -> Self {
        List(Arc::new(v))
    }
}

impl<T: Clone> From<&[T]> for List<T> {
    fn from(v: &[T]) -> Self {
        List(Arc::new(v.to_vec()))
    }
}

impl<T> FromIterator<T> for List<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        List(Arc::new(iter.into_iter().collect()))
    }
}

impl<T: Clone> IntoIterator for List<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        Arc::unwrap_or_clone(self.0).into_iter()
    }
}

impl<'a, T> IntoIterator for &'a List<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for List<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Clone> List<T> {
    /// This array's elements, then `other`'s. Appends in place when nothing
    /// else shares this array.
    pub fn concat(mut self, other: &List<T>) -> List<T> {
        Arc::make_mut(&mut self.0).extend_from_slice(other);
        self
    }
}

/// A map. Immutable, so a clone shares it. `merge` and `without` change
/// this one in place when nothing else shares it, and copy otherwise.
/// Derefs to a `BTreeMap`.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Map<K, V>(Arc<BTreeMap<K, V>>);

impl<K, V> Clone for Map<K, V> {
    fn clone(&self) -> Self {
        Map(self.0.clone())
    }
}

impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Map(Arc::new(BTreeMap::new()))
    }
}

impl<K, V> std::ops::Deref for Map<K, V> {
    type Target = BTreeMap<K, V>;
    fn deref(&self) -> &BTreeMap<K, V> {
        &self.0
    }
}

impl<K, V> From<BTreeMap<K, V>> for Map<K, V> {
    fn from(m: BTreeMap<K, V>) -> Self {
        Map(Arc::new(m))
    }
}

impl<K: Ord, V> FromIterator<(K, V)> for Map<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Map(Arc::new(iter.into_iter().collect()))
    }
}

impl<K: Clone, V: Clone> IntoIterator for Map<K, V> {
    type Item = (K, V);
    type IntoIter = std::collections::btree_map::IntoIter<K, V>;
    fn into_iter(self) -> Self::IntoIter {
        Arc::unwrap_or_clone(self.0).into_iter()
    }
}

impl<K: std::fmt::Debug, V: std::fmt::Debug> std::fmt::Debug for Map<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<K: Ord + Clone, V: Clone> Map<K, V> {
    /// This map's entries and `other`'s; `other` wins.
    pub fn merge(mut self, other: &Map<K, V>) -> Map<K, V> {
        Arc::make_mut(&mut self.0).extend(other.iter().map(|(k, v)| (k.clone(), v.clone())));
        self
    }

    /// This map without `keys`.
    pub fn without(mut self, keys: &[K]) -> Map<K, V> {
        if keys.iter().any(|k| self.0.contains_key(k)) {
            let m = Arc::make_mut(&mut self.0);
            for k in keys {
                m.remove(k);
            }
        }
        self
    }
}

"#;
