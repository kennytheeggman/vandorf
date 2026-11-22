//! Go backend.
//!
//! # Artifacts
//!
//! - **types** (`<types dir>/types_vandorf.go`, its own package by default):
//!   one named Go type per struct/enum shape in the [`TypeTable`]. Structs
//!   are structs with exported fields; enums are
//!   `struct{ Tag string; <Variant> <payload>; ... }`.
//! - **code** (the output package): the procs, their private input structs,
//!   stubs and runtime helpers. Imports the types package and the impls.
//!
//! The types live in a separate package so that implementations in other
//! packages can use them: the code package imports the implementations, so
//! implementations can't import the code package.
//!
//! # Model
//!
//! Sequential edges are tail calls, so a proc compiles to a state machine:
//! each node gets a program counter and an input and output slot in a
//! per-proc env struct, and an edge is "write the target's input slot, set
//! pc, continue". Back edges (`Loop -> Loop`) don't grow the stack. A
//! `for each` region (see [`crate::sema::Region`]) compiles to its own state
//! machine run once per element, concurrently, on a copy of the env; the
//! parent resumes at the collect target with the ordered results as `this`.
//!
//! # Implementations
//!
//! A func's `"file.go:Function"` reference names the Go function to call:
//!
//! ```go
//! func Function(ctx context.Context, <params in declared order>) (<ret>, error)
//! func Function(ctx context.Context, <params>) error // void funcs
//! ```
//!
//! The file is relative to the code package's directory. A file in that
//! directory is in the same package and is called directly; a file elsewhere
//! is imported (`<import base>/<dir>`). Funcs without a `.go` reference call a
//! generated stub that returns an error.
//!
//! # Main
//!
//! With [`GoOptions::main`], a `package main` in `<code dir>/main` runs proc
//! `Main`, parsing its parameters from `os.Args` in declared order and
//! printing its result.

use super::types::{Shape, TyRef, TypeTable};
use super::{Artifact, ImplUse, MainSpec, Output, Writer, dir_of, exported, gname, join_path, quote};
use crate::sema::{Arm, BinOp, Body, Call, Edge, Expr, NodeId, NodeKind, Pat, PatKind, Program, Query, Ref, RegionId, Root, Test, UnOp};
use crate::types::{Prim, Type};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

#[derive(Debug, Clone)]
pub struct GoOptions {
    /// Package name of the code artifact.
    pub package: String,
    /// Import path of the code package's directory. Needed to import the
    /// types package and impls in other directories.
    pub import_base: Option<String>,
    /// Directory of the types package, relative to the code package's
    /// directory. Empty or `.` puts the types in the code package.
    pub types_dir: String,
    /// Executables to generate, each a `package main` in its own directory.
    pub mains: Vec<MainSpec>,
}

impl GoOptions {
    pub fn new(package: impl Into<String>, import_base: Option<String>) -> GoOptions {
        GoOptions { package: package.into(), import_base, types_dir: "types".into(), mains: vec![] }
    }
}

pub fn generate(prog: &Program, opts: &GoOptions) -> Result<Output, String> {
    let table = TypeTable::build(prog);
    let separate = !matches!(opts.types_dir.trim_matches('/'), "" | ".");
    let (types_pkg, types_import) = if separate {
        let dir = opts.types_dir.trim_matches('/');
        let base = opts.import_base.as_deref().ok_or(
            "the types package needs an import path: pass --module, run inside a Go module, or put types in the code package (--types-dir .)",
        )?;
        let import = join_path(base, dir).ok_or_else(|| format!("cannot resolve import path for types dir {dir}"))?;
        (ident(import.rsplit('/').next().unwrap()), Some(import))
    } else {
        (opts.package.clone(), None)
    };
    let mut g = Gen {
        prog,
        opts,
        table: &table,
        tq: if separate { format!("{types_pkg}.") } else { String::new() },
        imports: BTreeMap::new(),
        coercions: HashMap::new(),
        helpers: vec![],
        stubs: BTreeSet::new(),
        failed: None,
        temps: 0,
        proc_name: String::new(),
    };
    g.check_names()?;

    let types = Artifact {
        dir: if separate { opts.types_dir.trim_matches('/').to_string() } else { String::new() },
        files: BTreeMap::from([("types_vandorf.go".to_string(), g.typegen(&types_pkg))]),
    };

    // Each file imports the types package only if its code (not comments)
    // mentions it; Go rejects unused imports.
    let file = |body: &str, mut imports: Vec<(String, Option<String>)>| {
        if let Some(ti) = &types_import
            && body.lines().any(|l| !l.trim_start().starts_with("//") && l.contains(&g_tq(&types_pkg)))
        {
            imports.push((ti.clone(), None));
        }
        let mut w = header(&opts.package, &imports);
        w.raw(body);
        w.out
    };

    let mut files = BTreeMap::new();
    for &p in &prog.top_procs {
        g.imports.clear();
        let mut body = Writer::new("\t");
        g.emit_proc_tree(&mut body, p)?;
        let mut imports = vec![("context".to_string(), None), ("fmt".to_string(), None)];
        imports.extend(g.imports.iter().map(|(path, alias)| (path.clone(), Some(alias.clone()))));
        files.insert(format!("{}_vandorf.go", prog.node(p).name().to_lowercase()), file(&body.out, imports));
    }

    let mut w = Writer::new("\t");
    for (id, n) in prog.nodes.iter().enumerate() {
        if let NodeKind::Func(_) | NodeKind::Id(_) = &n.kind {
            w.line(&format!("// {} is the input of {}.", g.in_type(id), n.qual_name()));
            w.line(&format!("type {} struct {{", g.in_type(id)));
            for (name, t) in prog.params(id) {
                w.line(&format!("{} {}", exported(name), g.go_type(t)));
            }
            w.line("}");
            w.line("");
        }
    }
    files.insert("inputs_vandorf.go".into(), file(&w.out, vec![]));

    if !g.stubs.is_empty() {
        let mut w = Writer::new("\t");
        for &n in &g.stubs.clone() {
            let decl = prog.node(n);
            let stub = format!("unimplemented{}", gname(prog, n));
            let msg = quote(&format!("{} is not implemented", decl.qual_name()));
            w.line(&format!("// {stub} stands in for {}, which names no .go implementation. Write", decl.qual_name()));
            w.line(&format!("//\t{}", g.signature(n, "Name", true)));
            w.line("// and add `= \"file.go:Name\"` to its declaration.");
            w.line(&format!("{} {{", g.signature(n, &stub, false)));
            let ret = prog.ret(n);
            if ret.is_void() {
                w.line(&format!("return errors.New({msg})"));
            } else {
                w.line(&format!("var zero {}", g.go_type(ret)));
                w.line(&format!("return zero, errors.New({msg})"));
            }
            w.line("}");
            w.line("");
        }
        let imports = vec![("context".to_string(), None), ("errors".to_string(), None)];
        files.insert("stubs_vandorf.go".into(), file(&w.out, imports));
    }

    let mut body = RUNTIME.to_string();
    for h in &g.helpers {
        body.push('\n');
        body.push_str(h);
    }
    let imports = ["cmp", "context", "fmt", "slices", "sync"].iter().map(|i| (i.to_string(), None)).collect();
    files.insert("runtime_vandorf.go".into(), file(&body, imports));

    if let Some(e) = g.failed.take() {
        return Err(e);
    }
    let impls = prog
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(id, n)| match &n.kind {
            NodeKind::Func(f) => f.imp("go").map(|i| ImplUse {
                id,
                node: n.qual_name(),
                file: i.file.clone(),
                function: i.function.clone(),
                signature: g.signature(id, &i.function, true),
            }),
            NodeKind::Proc(_) | NodeKind::Alias(_) | NodeKind::Id(_) => None,
        })
        .collect();
    let mains = opts.mains.iter().map(|m| main_artifact(prog, opts, m)).collect::<Result<_, _>>()?;
    Ok(Output { types, code: Artifact { dir: String::new(), files }, mains, impls })
}

fn main_artifact(prog: &Program, opts: &GoOptions, m: &MainSpec) -> Result<Artifact, String> {
    let (pid, params) = super::main_proc(prog, m)?;
    let import = opts
        .import_base
        .as_deref()
        .ok_or("the main package needs the code package's import path: pass --module or run inside a Go module")?;
    let alias = if opts.package == "main" { "procs".to_string() } else { opts.package.clone() };
    let usage = super::main_usage(&m.name, &params);

    let mut imports = vec![("context".to_string(), None), ("fmt".to_string(), None), ("os".to_string(), None)];
    let typed: &[(String, Prim)] = match &params {
        super::MainParams::Typed(ps) => ps,
        super::MainParams::Argv(_) => &[],
    };
    if typed.iter().any(|(_, p)| *p != Prim::String) {
        imports.push(("strconv".into(), None));
    }
    imports.push((import.to_string(), Some(alias.clone())));
    let mut w = header("main", &imports);
    // The doc comment has to sit right above `package`, which header() wrote.
    w.out = w.out.replacen(
        "package main",
        &format!("// Command {} runs proc {}, taking its parameters as arguments.\n//\n//\t{usage}\npackage main", m.name, m.entry),
        1,
    );
    w.line(&format!("const usage = {}", quote(&usage)));
    w.line("");
    w.line("func main() {");
    let mut args = vec!["context.Background()".to_string()];
    match &params {
        super::MainParams::Argv(_) => args.push("os.Args[1:]".into()),
        super::MainParams::Typed(ps) => {
            w.line(&format!("if len(os.Args) != {} {{", ps.len() + 1));
            w.line("fmt.Fprintln(os.Stderr, usage)");
            w.line("os.Exit(2)");
            w.line("}");
        }
    }
    for (i, (name, p)) in typed.iter().enumerate() {
        let var = format!("arg{}", exported(name));
        let raw = format!("os.Args[{}]", i + 1);
        let parse = match p {
            Prim::String => {
                w.line(&format!("{var} := {raw}"));
                args.push(var);
                continue;
            }
            Prim::Int => format!("strconv.Atoi({raw})"),
            Prim::Float => format!("strconv.ParseFloat({raw}, 64)"),
            Prim::Bool => format!("strconv.ParseBool({raw})"),
            Prim::Void => unreachable!(),
        };
        w.line(&format!("{var}, err := {parse}"));
        w.line("if err != nil {");
        w.line(&format!("fmt.Fprintf(os.Stderr, \"{name}: %v\\n%s\\n\", err, usage)"));
        w.line("os.Exit(2)");
        w.line("}");
        args.push(var);
    }
    let call = format!("{alias}.Run{}({})", gname(prog, pid), args.join(", "));
    let ret = prog.ret(pid);
    if ret.is_void() {
        w.line(&format!("if err := {call}; err != nil {{"));
    } else {
        w.line(&format!("result, err := {call}"));
        w.line("if err != nil {");
    }
    w.line("fmt.Fprintln(os.Stderr, \"error:\", err)");
    w.line("os.Exit(1)");
    w.line("}");
    match ret.unfold(&prog.types) {
        Type::Prim(Prim::Void) => {}
        Type::Prim(_) => w.line("fmt.Println(result)"),
        _ => w.line("fmt.Printf(\"%+v\\n\", result)"),
    }
    w.line("}");
    Ok(Artifact { dir: m.path.trim_matches('/').into(), files: BTreeMap::from([("main_vandorf.go".to_string(), w.out)]) })
}

fn g_tq(pkg: &str) -> String {
    format!("{pkg}.")
}

struct Gen<'a> {
    prog: &'a Program,
    opts: &'a GoOptions,
    table: &'a TypeTable,
    /// Qualifier for named types in the code package: `types.`, or empty when
    /// the types are in the code package.
    tq: String,
    /// Impl imports of the file being generated: path -> alias.
    imports: BTreeMap<String, String>,
    coercions: HashMap<(String, String), String>,
    helpers: Vec<String>,
    stubs: BTreeSet<NodeId>,
    /// Counter for temporaries holding call results in expressions.
    temps: usize,
    /// The proc being written, for error messages.
    proc_name: String,
    /// The first error found while writing an expression.
    failed: Option<String>,
}

impl Gen<'_> {
    // ---- Names ----

    /// Rejects programs whose names would collide or be invalid in Go.
    fn check_names(&self) -> Result<(), String> {
        let mut seen: HashMap<String, String> = HashMap::new();
        for (id, n) in self.prog.nodes.iter().enumerate() {
            let g = gname(self.prog, id);
            if let Some(prev) = seen.insert(g.clone(), n.qual_name()) {
                return Err(format!("{prev} and {} both map to the Go name {g}", n.qual_name()));
            }
            if let NodeKind::Func(f) = &n.kind {
                let names: Vec<&str> = f.params.iter().map(|(n, _)| n.as_str()).collect();
                check_fields(&names, false).map_err(|e| format!("{}: {e}", n.qual_name()))?;
            }
        }
        for t in &self.table.types {
            let (names, is_enum) = match &t.shape {
                Shape::Struct(fs) => (fs, false),
                Shape::Enum(vs) => (vs, true),
            };
            let names: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
            check_fields(&names, is_enum).map_err(|e| format!("{} ({}): {e}", t.name, t.origin))?;
        }
        Ok(())
    }

    fn in_type(&self, n: NodeId) -> String {
        match &self.prog.node(n).kind {
            NodeKind::Func(_) | NodeKind::Id(_) => format!("in{}", gname(self.prog, n)),
            NodeKind::Proc(p) => self.in_type(p.entry),
            NodeKind::Alias(b) => self.in_type(*b),
        }
    }

    fn signature(&self, n: NodeId, name: &str, named: bool) -> String {
        let mut s = format!("func {name}(ctx context.Context");
        for (p, t) in self.prog.params(n) {
            let pname = if named { go_param(p) } else { "_".into() };
            write!(s, ", {pname} {}", self.go_type(t)).unwrap();
        }
        let ret = self.prog.ret(n);
        if ret.is_void() {
            s.push_str(") error");
        } else {
            write!(s, ") ({}, error)", self.go_type(ret)).unwrap();
        }
        s
    }

    /// The Go function a func node (or an alias of one) calls.
    fn callee(&mut self, n: NodeId) -> Result<String, String> {
        let n = self.prog.base(n);
        let NodeKind::Func(f) = &self.prog.node(n).kind else { unreachable!() };
        let Some(imp) = f.imp("go") else {
            self.stubs.insert(n);
            return Ok(format!("unimplemented{}", gname(self.prog, n)));
        };
        let dir = dir_of(&imp.file).trim_end_matches('/');
        if dir.is_empty() || dir == "." {
            return Ok(imp.function.clone());
        }
        let qual = self.prog.node(n).qual_name();
        let Some(base) = &self.opts.import_base else {
            return Err(format!(
                "{qual} is implemented in {}, outside the code package, but its import path is unknown (pass --module or run inside a Go module)",
                imp.file,
            ));
        };
        let path = join_path(base, dir)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| format!("{qual}: {} is outside the Go module {base} is in", imp.file))?;
        let alias = match self.imports.get(&path) {
            Some(a) => a.clone(),
            None => {
                let stem = ident(path.rsplit('/').next().unwrap());
                let taken = |a: &str, imports: &BTreeMap<String, String>| {
                    imports.values().any(|x| x == a) || ["context", "fmt"].contains(&a) || g_tq(a) == self.tq
                };
                let mut alias = stem.clone();
                let mut i = 2;
                while taken(&alias, &self.imports) {
                    alias = format!("{stem}{i}");
                    i += 1;
                }
                self.imports.insert(path, alias.clone());
                alias
            }
        };
        Ok(format!("{alias}.{}", imp.function))
    }

    // ---- Types ----

    fn typegen(&self, pkg: &str) -> String {
        let mut w = header(pkg, &[]);
        for (i, t) in self.table.types.iter().enumerate() {
            if t.origin == format!("typedef {}", t.name) {
                w.line(&format!("// {} is a typedef.", t.name));
            } else {
                w.line(&format!("// {} is the type of the {}.", t.name, t.origin));
            }
            match &t.shape {
                Shape::Struct(fs) => {
                    w.line(&format!("type {} struct {{", t.name));
                    for (n, r) in fs {
                        w.line(&format!("{} {}", exported(n), self.field_type(i, n, r)));
                    }
                }
                Shape::Enum(vs) => {
                    let tags: Vec<String> = vs.iter().map(|(n, _)| quote(n)).collect();
                    w.line(&format!("// Tag is one of {}; the field it names holds the payload.", tags.join(", ")));
                    w.line(&format!("type {} struct {{", t.name));
                    w.line("Tag string");
                    for (n, r) in vs {
                        w.line(&format!("{} {}", exported(n), self.field_type(i, n, r)));
                    }
                }
            }
            w.line("}");
            w.line("");
        }
        for (name, r) in &self.table.aliases {
            w.line(&format!("// {name} is a typedef."));
            w.line(&format!("type {name} = {}", self.render(r, "")));
            w.line("");
        }
        w.out
    }

    /// Field `n` of type `i`, a pointer where it would otherwise make the
    /// type contain itself.
    fn field_type(&self, i: usize, n: &str, r: &TyRef) -> String {
        let t = self.render(r, "");
        if self.table.is_boxed(i, n) { format!("*{t}") } else { t }
    }

    fn render(&self, r: &TyRef, qual: &str) -> String {
        match r {
            TyRef::Prim(p) => match p {
                Prim::Void => "struct{}",
                Prim::Int => "int",
                Prim::Float => "float64",
                Prim::Bool => "bool",
                Prim::String => "string",
            }
            .into(),
            TyRef::Array(e) => format!("[]{}", self.render(e, qual)),
            TyRef::Map(k, v) => format!("map[{}]{}", self.render(k, qual), self.render(v, qual)),
            TyRef::Named(i) => format!("{qual}{}", self.table.types[*i].name),
        }
    }

    /// A vandorf type as written in the code package.
    fn go_type(&self, t: &Type) -> String {
        self.render(&self.table.get(t), &self.tq)
    }

    /// Converts `expr` of type `from` to `to`, which sema checked accepts it.
    fn coerce(&mut self, expr: &str, from: &Type, to: &Type) -> String {
        let (src, dst) = (self.go_type(from), self.go_type(to));
        if src == dst {
            return expr.to_string();
        }
        let key = (src.clone(), dst.clone());
        if let Some(name) = self.coercions.get(&key) {
            return format!("{name}({expr})");
        }
        let name = format!("coerce{}", self.coercions.len());
        self.coercions.insert(key, name.clone());
        let mut w = Writer::new("\t");
        w.line(&format!("func {name}(v {src}) (r {dst}) {{"));
        let prog = self.prog;
        let defs = &prog.types;
        let (si, di) = (self.table.index(from), self.table.index(to));
        // Copies part `n` (a field, or the payload of variant `n`), through
        // pointers where either side boxes it.
        let part = |g: &mut Self, w: &mut Writer, n: &str, f: &Type, t: &Type| {
            let fe = exported(n);
            let sb = si.is_some_and(|i| g.table.is_boxed(i, n));
            let db = di.is_some_and(|i| g.table.is_boxed(i, n));
            let e = g.coerce(&if sb { format!("(*v.{fe})") } else { format!("v.{fe}") }, f, t);
            let e = if db { format!("ptrTo({e})") } else { e };
            if sb {
                w.line(&format!("if v.{fe} != nil {{"));
            }
            w.line(&format!("r.{fe} = {e}"));
            if sb {
                w.line("}");
            }
        };
        match (from.unfold(defs), to.unfold(defs)) {
            (Type::Array(f), Type::Array(t)) => {
                w.line(&format!("r = make({dst}, len(v))"));
                w.line("for i := range v {");
                let e = self.coerce("v[i]", f, t);
                w.line(&format!("r[i] = {e}"));
                w.line("}");
            }
            (Type::Map(_, f), Type::Map(_, t)) => {
                w.line(&format!("r = make({dst}, len(v))"));
                w.line("for k, x := range v {");
                let e = self.coerce("x", f, t);
                w.line(&format!("r[k] = {e}"));
                w.line("}");
            }
            (Type::Struct(_), Type::Struct(tfs)) => {
                for (n, t) in tfs {
                    part(self, &mut w, n, from.field(n, defs).unwrap(), t);
                }
            }
            (Type::Enum(fvs), Type::Enum(_)) => {
                w.line("r.Tag = v.Tag");
                w.line("switch v.Tag {");
                for (n, f) in fvs {
                    w.line(&format!("case {}:", quote(n)));
                    if !f.is_void() {
                        part(self, &mut w, n, f, to.variant(n, defs).unwrap());
                    }
                }
                w.line("}");
            }
            _ => unreachable!("sema admitted coercion {src} -> {dst}"),
        }
        w.line("return r");
        w.line("}");
        self.helpers.push(w.out);
        format!("{name}({expr})")
    }

    // ---- Expressions ----

    fn reference(&self, r: &Ref) -> String {
        let mut s = match r.root {
            Root::This => "this".to_string(),
            Root::Node(n) => format!("env.out{}", gname(self.prog, n)),
            Root::Bind => bind_var(r.bind.as_deref().unwrap()),
        };
        // Go selects through pointers implicitly; only a boxed last field
        // needs dereferencing.
        let mut ty = &r.root_ty;
        let mut boxed = false;
        for f in &r.fields {
            boxed = self.table.index(ty).is_some_and(|i| self.table.is_boxed(i, f));
            ty = ty.field(f, &self.prog.types).unwrap();
            write!(s, ".{}", exported(f)).unwrap();
        }
        if boxed { format!("(*{s})") } else { s }
    }

    /// `e` as a Go expression of type `want`. Calls in `e` can fail, so
    /// they're evaluated by statements written to `w` first, which return
    /// from the enclosing function on error.
    fn value(&mut self, w: &mut Writer, e: &Expr, want: &Type) -> String {
        let prog = self.prog;
        let defs = &prog.types;
        let boxed = |g: &Self, part: &str, v: String| match g.table.index(want) {
            Some(i) if g.table.is_boxed(i, part) => format!("ptrTo({v})"),
            _ => v,
        };
        match e {
            Expr::Ref(r) => self.coerce(&self.reference(r), &r.ty, want),
            Expr::Call(c) => {
                let v = self.own(w, e);
                self.coerce(&v, prog.ret(c.target), want)
            }
            Expr::Array(es) => {
                let Type::Array(elem) = want.unfold(defs) else { unreachable!() };
                let items: Vec<String> = es.iter().map(|e| self.value(w, e, elem)).collect();
                format!("{}{{{}}}", self.go_type(want), items.join(", "))
            }
            Expr::Map(kvs) => {
                let Type::Map(kt, vt) = want.unfold(defs) else { unreachable!() };
                let mut items = vec![];
                for (k, v) in kvs {
                    let k = self.value(w, k, kt);
                    let v = self.value(w, v, vt);
                    items.push(format!("{k}: {v}"));
                }
                format!("{}{{{}}}", self.go_type(want), items.join(", "))
            }
            Expr::Index { ty, .. } => {
                let v = self.own(w, e);
                let vt = match ty.unfold(defs) {
                    Type::Map(_, v) | Type::Array(v) => v,
                    _ => unreachable!(),
                };
                self.coerce(&v, vt, want)
            }
            Expr::Field { ty, name, .. } => {
                let v = self.own(w, e);
                self.coerce(&v, ty.field(name, defs).unwrap(), want)
            }
            Expr::Merge { ty, .. } | Expr::Remove { ty, .. } => {
                let v = self.own(w, e);
                self.coerce(&v, ty, want)
            }
            Expr::Query { ty, q: q @ (Query::Keys | Query::Values), .. } => {
                let v = self.own(w, e);
                let Type::Map(k, v_) = ty.unfold(defs) else { unreachable!() };
                let item = if *q == Query::Keys { k } else { v_ };
                self.coerce(&v, &Type::Array(item.clone()), want)
            }
            Expr::Struct(fs) => {
                let mut items = vec![];
                for (n, e) in fs {
                    let v = self.value(w, e, want.field(n, defs).unwrap());
                    items.push(format!("{}: {}", exported(n), boxed(self, n, v)));
                }
                format!("{}{{{}}}", self.go_type(want), items.join(", "))
            }
            Expr::Variant { tag, payload } => {
                let mut s = format!("{}{{Tag: {}", self.go_type(want), quote(tag));
                if let Some(p) = payload {
                    let v = self.value(w, p, want.variant(tag, defs).unwrap());
                    write!(s, ", {}: {}", exported(tag), boxed(self, tag, v)).unwrap();
                }
                s + "}"
            }
            // Primitives, whose Go types match whenever sema accepted them.
            _ => self.own(w, e),
        }
    }

    /// An expression that has its own type (not a struct, array or variant
    /// literal), at that type.
    fn own(&mut self, w: &mut Writer, e: &Expr) -> String {
        let prog = self.prog;
        match e {
            Expr::Ref(r) => self.reference(r),
            Expr::Int(i) => i.to_string(),
            Expr::Float(f) => format!("{f:?}"),
            Expr::Str(s) => quote(s),
            Expr::Bool(b) => b.to_string(),
            Expr::Unary(op, x) => {
                let x = self.own(w, x);
                format!("({}{x})", if *op == UnOp::Not { "!" } else { "-" })
            }
            Expr::Binary { op: op @ (BinOp::And | BinOp::Or), l, r, .. } if r.can_fail() => {
                // Keep short-circuiting: the right side's calls only run if needed.
                let l = self.own(w, l);
                let t = self.temp();
                w.line(&format!("{t} := {l}"));
                w.line(&if *op == BinOp::And { format!("if {t} {{") } else { format!("if !{t} {{") });
                let r = self.own(w, r);
                w.line(&format!("{t} = {r}"));
                w.line("}");
                t
            }
            Expr::Binary { op, l, r, .. } => {
                let (l, r) = (self.own(w, l), self.own(w, r));
                format!("({l} {} {r})", op.symbol())
            }
            Expr::Call(c) => {
                let callee = self.callee(c.target).unwrap_or_else(|e| {
                    // Reported by generate() once the expression is written.
                    self.failed.get_or_insert(e);
                    "unresolved".into()
                });
                let mut call = format!("{callee}(ctx");
                for a in &c.args {
                    let v = self.value(w, &a.value, &a.param);
                    write!(call, ", {v}").unwrap();
                }
                call.push(')');
                let t = self.temp();
                w.line(&format!("{t}, err := {call}"));
                w.line("if err != nil {");
                w.line(&format!("return zero, fmt.Errorf({}, err)", quote(&format!("{}: %w", prog.node(c.target).qual_name()))));
                w.line("}");
                t
            }
            Expr::Index { e, key, ty } => {
                let (x, k) = (self.own(w, e), self.own(w, key));
                let get = if matches!(ty.unfold(&prog.types), Type::Map(..)) { "mapGet" } else { "sliceGet" };
                let t = self.temp();
                w.line(&format!("{t}, err := {get}({x}, {k})"));
                w.line("if err != nil {");
                w.line(&format!("return zero, fmt.Errorf({}, err)", quote(&format!("{}: %w", self.proc_name))));
                w.line("}");
                t
            }
            Expr::Field { e, ty, name } => {
                // The struct may be a literal (`{ ..x, f = v }.g`).
                let x = self.value(w, e, ty);
                let v = format!("({x}).{}", exported(name));
                if self.table.index(ty).is_some_and(|i| self.table.is_boxed(i, name)) { format!("(*{v})") } else { v }
            }
            Expr::In { key, map } => {
                let (k, m) = (self.own(w, key), self.own(w, map));
                format!("mapHas({m}, {k})")
            }
            Expr::Query { e, q, .. } => {
                let x = self.own(w, e);
                match q {
                    Query::Len => format!("len({x})"),
                    Query::Keys => format!("mapKeys({x})"),
                    Query::Values => format!("mapValues({x})"),
                }
            }
            Expr::Merge { l, r, ty } => {
                let l = self.value(w, l, ty);
                let r = self.value(w, r, ty);
                let f = if matches!(ty.unfold(&prog.types), Type::Array(_)) { "sliceConcat" } else { "mapMerge" };
                format!("{f}({l}, {r})")
            }
            Expr::Remove { m, keys, ty } => {
                let Type::Map(k, _) = ty.unfold(&prog.types) else { unreachable!() };
                let m = self.own(w, m);
                let ks = self.value(w, keys, &Type::Array(k.clone()));
                format!("mapRemove({m}, {ks})")
            }
            Expr::Struct(_) | Expr::Array(_) | Expr::Variant { .. } | Expr::Map(_) => unreachable!("literal without a type"),
        }
    }

    fn temp(&mut self) -> String {
        self.temps += 1;
        format!("v{}", self.temps)
    }

    fn input(&mut self, w: &mut Writer, c: &Call) -> String {
        let mut parts = vec![];
        for a in &c.args {
            let v = self.value(w, &a.value, &a.param);
            parts.push(format!("{}: {v}", exported(&a.name)));
        }
        format!("{}{{{}}}", self.in_type(c.target), parts.join(", "))
    }

    // ---- Procs ----

    fn emit_proc_tree(&mut self, w: &mut Writer, p: NodeId) -> Result<(), String> {
        self.emit_proc(w, p)?;
        for &l in &self.prog.proc(p).locals {
            if let NodeKind::Proc(_) = self.prog.node(l).kind {
                self.emit_proc_tree(w, l)?;
            }
        }
        Ok(())
    }

    fn emit_proc(&mut self, w: &mut Writer, pid: NodeId) -> Result<(), String> {
        let prog = self.prog;
        self.proc_name = prog.node(pid).qual_name();
        let proc = prog.proc(pid);
        let g = &proc.graph;
        let name = gname(prog, pid);
        let env = format!("env{name}");
        let order: Vec<NodeId> = g.regions.iter().flat_map(|r| r.nodes.iter().copied()).collect();

        w.line(&format!("type {env} struct {{"));
        for &n in &order {
            let id = gname(prog, n);
            w.line(&format!("in{id} {}", self.in_type(n)));
            w.line(&format!("out{id} {}", self.go_type(prog.ret(n))));
        }
        w.line("}");
        w.line("");
        w.line("const (");
        for (i, &n) in order.iter().enumerate() {
            let pc = pc_name(prog, pid, n);
            w.line(&if i == 0 { format!("{pc} = iota") } else { pc });
        }
        w.line(")");
        w.line("");

        let params = prog.params(pid);
        let ret = self.go_type(&proc.ret);
        let sig: String = params.iter().map(|(p, t)| format!(", {} {}", go_param(p), self.go_type(t))).collect();
        w.line(&format!("// Run{name} runs proc {}.", prog.node(pid).qual_name()));
        w.line(&format!("func Run{name}(ctx context.Context{sig}) ({ret}, error) {{"));
        w.line(&format!("var zero {ret}"));
        w.line(&format!("env := &{env}{{}}"));
        let fields: Vec<String> = params.iter().map(|(p, _)| format!("{}: {}", exported(p), go_param(p))).collect();
        w.line(&format!("env.in{} = {}{{{}}}", gname(prog, proc.entry), self.in_type(proc.entry), fields.join(", ")));
        w.line(&format!("pc := {}", pc_name(prog, pid, proc.entry)));
        self.emit_loop(w, pid, 0)?;
        w.line("}");
        w.line("");

        let mut opens: Vec<(NodeId, RegionId)> = g.opens.iter().map(|(&n, &r)| (n, r)).collect();
        opens.sort_by_key(|&(_, r)| r);
        for (opener, r) in opens {
            let region = &g.regions[r];
            let Some(Edge::ForEach { call, .. }) = g.edges.get(&opener) else { unreachable!() };
            let sink = self.go_type(prog.ret(region.sink.unwrap()));
            let f = region_fn(prog, pid, r);
            w.line(&format!("// {f} runs one element of the for each from {}.", prog.node(opener).qual_name()));
            w.line(&format!("func {f}(ctx context.Context, env *{env}, elem {}) ({sink}, error) {{", self.go_type(&region.elem)));
            w.line(&format!("var zero {sink}"));
            w.line("this := elem");
            w.line("_ = this");
            let input = self.input(w, call);
            w.line(&format!("env.in{} = {input}", gname(prog, region.entry)));
            w.line(&format!("pc := {}", pc_name(prog, pid, region.entry)));
            self.emit_loop(w, pid, r)?;
            w.line("}");
            w.line("");
        }
        Ok(())
    }

    fn emit_loop(&mut self, w: &mut Writer, pid: NodeId, r: RegionId) -> Result<(), String> {
        w.line("for {");
        w.line("if err := ctx.Err(); err != nil {");
        w.line("return zero, err");
        w.line("}");
        w.line("switch pc {");
        for &n in &self.prog.proc(pid).graph.regions[r].nodes {
            self.emit_node(w, pid, n)?;
        }
        w.line("default:");
        let msg = quote(&format!("{}: invalid state %d", self.prog.node(pid).qual_name()));
        w.line(&format!("return zero, fmt.Errorf({msg}, pc)"));
        w.line("}");
        w.line("}");
        Ok(())
    }

    fn emit_node(&mut self, w: &mut Writer, pid: NodeId, n: NodeId) -> Result<(), String> {
        let prog = self.prog;
        let proc = prog.proc(pid);
        let id = gname(prog, n);
        let qual = prog.node(n).qual_name();
        let ret = prog.ret(n);
        let base = prog.base(n);
        let mut call = match prog.node(base).kind {
            NodeKind::Proc(_) => format!("Run{}", gname(prog, base)),
            NodeKind::Id(_) => String::new(),
            _ => self.callee(n)?,
        };
        call.push_str("(ctx");
        for (p, _) in prog.params(n) {
            write!(call, ", env.in{id}.{}", exported(p)).unwrap();
        }
        call.push(')');

        w.line(&format!("case {}:", pc_name(prog, pid, n)));
        if let NodeKind::Id(_) = prog.node(n).kind {
            // id: the value is the output.
            w.line(&format!("this := env.in{id}.Value"));
        } else if matches!(prog.node(base).kind, NodeKind::Func(_)) && ret.is_void() {
            w.line(&format!("err := {call}"));
            w.line("this := struct{}{}");
        } else {
            w.line(&format!("this, err := {call}"));
        }
        if !matches!(prog.node(n).kind, NodeKind::Id(_)) {
            w.line("if err != nil {");
            w.line(&format!("return zero, fmt.Errorf({}, err)", quote(&format!("{qual}: %w"))));
            w.line("}");
        }
        w.line(&format!("env.out{id} = this"));

        let Some(edge) = proc.graph.edges.get(&n) else {
            let e = self.coerce("this", ret, &proc.ret);
            w.line(&format!("return {e}, nil"));
            return Ok(());
        };
        match edge {
            Edge::Call(c) => self.transition(w, pid, c),
            Edge::Collect(_) => w.line("return this, nil"),
            Edge::Match(arms) => self.emit_switch(w, pid, n, arms, true),
            Edge::ForEach { dim, .. } => {
                let r = proc.graph.opens[&n];
                let region = &proc.graph.regions[r];
                let sink = region.sink.unwrap();
                let out = self.go_type(prog.ret(sink));
                let dim = self.own(w, dim);
                w.line(&format!(
                    "results, err := fanOut(ctx, {dim}, func(ctx context.Context, elem {}) ({out}, error) {{",
                    self.go_type(&region.elem),
                ));
                w.line("env := *env");
                w.line(&format!("return {}(ctx, &env, elem)", region_fn(prog, pid, r)));
                w.line("})");
                w.line("if err != nil {");
                w.line("return zero, err");
                w.line("}");
                w.line("{");
                w.line("this := results");
                w.line("_ = this");
                let Some(Edge::Collect(c)) = proc.graph.edges.get(&sink) else { unreachable!() };
                self.transition(w, pid, c);
                w.line("}");
            }
        }
        Ok(())
    }

    /// Arms in order; a transition ends with `continue`, so the first arm
    /// that matches is taken. Sema checked that one always does.
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
                    w.line("{");
                    let c = self.own(w, c);
                    w.line(&format!("if {c} {{"));
                    self.emit_body(w, pid, n, &a.body);
                    w.line("}");
                    w.line("}");
                }
                Test::Pat(p) => {
                    let (mut tests, mut binds) = (vec![], vec![]);
                    self.pat(p, "this", &mut tests, &mut binds);
                    always = tests.is_empty();
                    w.line(&if always { "{".to_string() } else { format!("if {} {{", tests.join(" && ")) });
                    for (b, v) in binds {
                        w.line(&format!("{} := {v}", bind_var(&b)));
                        w.line(&format!("_ = {}", bind_var(&b)));
                    }
                    self.emit_body(w, pid, n, &a.body);
                    w.line("}");
                }
            }
        }
        if top && !always {
            w.line(&format!("return zero, fmt.Errorf({}, this)", quote(&format!("{qual}: no match arm for %v"))));
        }
    }

    fn emit_body(&mut self, w: &mut Writer, pid: NodeId, n: NodeId, body: &Body) {
        match body {
            Body::Call(c) => self.transition(w, pid, c),
            Body::Switch(arms) => self.emit_switch(w, pid, n, arms, false),
        }
    }

    /// The tests matching `p` against the Go value `x` (in an order that
    /// checks a tag before reading its payload) and the bindings' values.
    fn pat(&self, p: &Pat, x: &str, tests: &mut Vec<String>, binds: &mut Vec<(String, String)>) {
        let boxed = |part: &str| self.table.index(&p.ty).is_some_and(|i| self.table.is_boxed(i, part));
        let part = |name: &str| {
            let v = format!("{x}.{}", exported(name));
            if boxed(name) { format!("(*{v})") } else { v }
        };
        match &p.kind {
            PatKind::Wild => {}
            PatKind::Bind(b) => binds.push((b.clone(), x.to_string())),
            PatKind::Variant { tag, payload } => {
                tests.push(format!("{x}.Tag == {}", quote(tag)));
                if let Some(q) = payload {
                    self.pat(q, &part(tag), tests, binds);
                }
            }
            PatKind::Struct(fs) => {
                for (f, q) in fs {
                    self.pat(q, &part(f), tests, binds);
                }
            }
            PatKind::Or(alts) => {
                let alts: Vec<String> = alts
                    .iter()
                    .map(|a| {
                        let mut t = vec![];
                        self.pat(a, x, &mut t, &mut vec![]);
                        if t.is_empty() { "true".to_string() } else { format!("({})", t.join(" && ")) }
                    })
                    .collect();
                tests.push(format!("({})", alts.join(" || ")));
            }
        }
    }

    fn transition(&mut self, w: &mut Writer, pid: NodeId, c: &Call) {
        let input = self.input(w, c);
        w.line(&format!("env.in{} = {input}", gname(self.prog, c.target)));
        w.line(&format!("pc = {}", pc_name(self.prog, pid, c.target)));
        w.line("continue");
    }
}

/// The Go variable of a switch-arm binding.
fn bind_var(name: &str) -> String {
    format!("b_{name}")
}

fn pc_name(prog: &Program, pid: NodeId, n: NodeId) -> String {
    format!("pc{}_{}", gname(prog, pid), gname(prog, n))
}

fn region_fn(prog: &Program, pid: NodeId, r: RegionId) -> String {
    format!("run{}Region{r}", gname(prog, pid))
}

fn check_fields(names: &[&str], is_enum: bool) -> Result<(), String> {
    let mut seen: HashMap<String, &str> = HashMap::new();
    if is_enum {
        seen.insert("Tag".into(), "the enum tag");
    }
    for n in names {
        if let Some(prev) = seen.insert(exported(n), n) {
            return Err(format!("{prev} and {n} collide as Go field {}", exported(n)));
        }
    }
    Ok(())
}

/// A Go identifier from a path segment.
fn ident(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if s.starts_with(|c: char| c.is_ascii_digit()) { format!("p{s}") } else { s }
}

const GO_RESERVED: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for",
    "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return",
    "select", "struct", "switch", "type", "var", "bool", "int", "float64", "string", "error",
    "any", "ctx", "env", "pc", "zero", "this", "err", "elem", "results",
];

fn go_param(s: &str) -> String {
    if GO_RESERVED.contains(&s) { format!("{s}_") } else { s.to_string() }
}

fn header(pkg: &str, imports: &[(String, Option<String>)]) -> Writer {
    let mut w = Writer::new("\t");
    w.line("// Code generated by vandorf. DO NOT EDIT.");
    w.line("");
    w.line(&format!("package {pkg}"));
    w.line("");
    if !imports.is_empty() {
        w.line("import (");
        for (path, alias) in imports {
            match alias {
                Some(a) => w.line(&format!("{a} {}", quote(path))),
                None => w.line(&quote(path)),
            }
        }
        w.line(")");
        w.line("");
    }
    w
}

const RUNTIME: &str = r#"// mapGet looks up k, failing if it's missing.
func mapGet[K comparable, V any](m map[K]V, k K) (V, error) {
	v, ok := m[k]
	if !ok {
		return v, fmt.Errorf("key %v not found", k)
	}
	return v, nil
}

// sliceGet indexes xs, failing if i is out of range.
func sliceGet[E any](xs []E, i int) (E, error) {
	if i < 0 || i >= len(xs) {
		var zero E
		return zero, fmt.Errorf("index %d out of range [0, %d)", i, len(xs))
	}
	return xs[i], nil
}

func mapHas[K comparable, V any](m map[K]V, k K) bool {
	_, ok := m[k]
	return ok
}

// mapKeys returns m's keys in order.
func mapKeys[K cmp.Ordered, V any](m map[K]V) []K {
	ks := make([]K, 0, len(m))
	for k := range m {
		ks = append(ks, k)
	}
	slices.Sort(ks)
	return ks
}

// mapValues returns m's values in key order.
func mapValues[K cmp.Ordered, V any](m map[K]V) []V {
	vs := make([]V, 0, len(m))
	for _, k := range mapKeys(m) {
		vs = append(vs, m[k])
	}
	return vs
}

// mapMerge returns a new map with a's entries and b's, b winning. Maps
// are values in vandorf, so the arguments are never changed.
func mapMerge[K comparable, V any](a, b map[K]V) map[K]V {
	r := make(map[K]V, len(a)+len(b))
	for k, v := range a {
		r[k] = v
	}
	for k, v := range b {
		r[k] = v
	}
	return r
}

// sliceConcat returns a new slice with a's elements, then b's.
func sliceConcat[E any](a, b []E) []E {
	r := make([]E, 0, len(a)+len(b))
	return append(append(r, a...), b...)
}

// mapRemove returns a new map without the keys ks.
func mapRemove[K comparable, V any](m map[K]V, ks []K) map[K]V {
	r := make(map[K]V, len(m))
	for k, v := range m {
		r[k] = v
	}
	for _, k := range ks {
		delete(r, k)
	}
	return r
}

// ptrTo boxes a value, for fields that would otherwise make a type contain
// itself.
func ptrTo[T any](v T) *T {
	return &v
}

// fanOut runs f over xs concurrently and returns the results in input order.
// The first error cancels the remaining elements.
func fanOut[E, R any](ctx context.Context, xs []E, f func(context.Context, E) (R, error)) ([]R, error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	out := make([]R, len(xs))
	var (
		wg    sync.WaitGroup
		once  sync.Once
		first error
	)
	for i, x := range xs {
		wg.Add(1)
		go func() {
			defer wg.Done()
			r, err := f(ctx, x)
			if err != nil {
				once.Do(func() { first = err; cancel() })
				return
			}
			out[i] = r
		}()
	}
	wg.Wait()
	return out, first
}
"#;
