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

use super::types::{Shape, TyRef, TypeTable};
use super::{Artifact, ImplUse, Output, Writer, dir_of, exported, gname, join_path, quote};
use crate::sema::{Call, Edge, Expr, NodeId, NodeKind, Program, Ref, RegionId, Root};
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
}

impl GoOptions {
    pub fn new(package: impl Into<String>, import_base: Option<String>) -> GoOptions {
        GoOptions { package: package.into(), import_base, types_dir: "types".into() }
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
        if let NodeKind::Func(f) = &n.kind {
            w.line(&format!("// {} is the input of {}.", g.in_type(id), n.qual_name()));
            w.line(&format!("type {} struct {{", g.in_type(id)));
            for (name, t) in &f.params {
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
    let imports = vec![("context".to_string(), None), ("sync".to_string(), None)];
    files.insert("runtime_vandorf.go".into(), file(&body, imports));

    let impls = prog
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(id, n)| match &n.kind {
            NodeKind::Func(f) => f.imp("go").map(|i| ImplUse {
                node: n.qual_name(),
                file: i.file.clone(),
                function: i.function.clone(),
                signature: g.signature(id, &i.function, true),
            }),
            NodeKind::Proc(_) => None,
        })
        .collect();
    Ok(Output { types, code: Artifact { dir: String::new(), files }, impls })
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
            NodeKind::Func(_) => format!("in{}", gname(self.prog, n)),
            NodeKind::Proc(p) => self.in_type(p.entry),
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

    /// The Go function a func node calls.
    fn callee(&mut self, n: NodeId) -> Result<String, String> {
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
        let path = join_path(base, dir).ok_or_else(|| format!("{qual}: cannot resolve import for {}", imp.file))?;
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
        for t in &self.table.types {
            w.line(&format!("// {} is the type of the {}.", t.name, t.origin));
            match &t.shape {
                Shape::Struct(fs) => {
                    w.line(&format!("type {} struct {{", t.name));
                    for (n, r) in fs {
                        w.line(&format!("{} {}", exported(n), self.render(r, "")));
                    }
                }
                Shape::Enum(vs) => {
                    let tags: Vec<String> = vs.iter().map(|(n, _)| quote(n)).collect();
                    w.line(&format!("// Tag is one of {}; the field it names holds the payload.", tags.join(", ")));
                    w.line(&format!("type {} struct {{", t.name));
                    w.line("Tag string");
                    for (n, r) in vs {
                        w.line(&format!("{} {}", exported(n), self.render(r, "")));
                    }
                }
            }
            w.line("}");
            w.line("");
        }
        w.out
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
        match (from, to) {
            (Type::Array(f), Type::Array(t)) => {
                w.line(&format!("r = make({dst}, len(v))"));
                w.line("for i := range v {");
                let e = self.coerce("v[i]", f, t);
                w.line(&format!("r[i] = {e}"));
                w.line("}");
            }
            (Type::Struct(_), Type::Struct(tfs)) => {
                for (n, t) in tfs {
                    let f = exported(n);
                    let e = self.coerce(&format!("v.{f}"), from.field(n).unwrap(), t);
                    w.line(&format!("r.{f} = {e}"));
                }
            }
            (Type::Enum(fvs), Type::Enum(_)) => {
                w.line("r.Tag = v.Tag");
                w.line("switch v.Tag {");
                for (n, f) in fvs {
                    let v = exported(n);
                    w.line(&format!("case {}:", quote(n)));
                    let e = self.coerce(&format!("v.{v}"), f, to.variant(n).unwrap());
                    w.line(&format!("r.{v} = {e}"));
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
        };
        for f in &r.fields {
            write!(s, ".{}", exported(f)).unwrap();
        }
        s
    }

    fn input(&mut self, c: &Call) -> String {
        let mut parts = vec![];
        for a in &c.args {
            let v = match &a.value {
                Expr::Ref(r) => self.coerce(&self.reference(r), &r.ty, &a.param),
                Expr::Variant { tag, payload } => {
                    let pt = a.param.variant(tag).unwrap();
                    let p = self.coerce(&self.reference(payload), &payload.ty, pt);
                    format!("{}{{Tag: {}, {}: {p}}}", self.go_type(&a.param), quote(tag), exported(tag))
                }
            };
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
            let input = self.input(call);
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
        let mut call = match prog.node(n).kind {
            NodeKind::Func(_) => self.callee(n)?,
            NodeKind::Proc(_) => format!("Run{id}"),
        };
        call.push_str("(ctx");
        for (p, _) in prog.params(n) {
            write!(call, ", env.in{id}.{}", exported(p)).unwrap();
        }
        call.push(')');

        w.line(&format!("case {}:", pc_name(prog, pid, n)));
        if matches!(prog.node(n).kind, NodeKind::Func(_)) && ret.is_void() {
            w.line(&format!("err := {call}"));
            w.line("this := struct{}{}");
        } else {
            w.line(&format!("this, err := {call}"));
        }
        w.line("if err != nil {");
        w.line(&format!("return zero, fmt.Errorf({}, err)", quote(&format!("{qual}: %w"))));
        w.line("}");
        w.line(&format!("env.out{id} = this"));

        let Some(edge) = proc.graph.edges.get(&n) else {
            let e = self.coerce("this", ret, &proc.ret);
            w.line(&format!("return {e}, nil"));
            return Ok(());
        };
        match edge {
            Edge::Call(c) => self.transition(w, pid, c),
            Edge::Collect(_) => w.line("return this, nil"),
            Edge::Match(arms) => {
                let is_enum = matches!(ret, Type::Enum(_));
                w.line(if is_enum { "switch this.Tag {" } else { "switch {" });
                for (cond, c) in arms {
                    if cond == "else" {
                        w.line("default:");
                    } else if is_enum {
                        if ret.variant(cond).is_none() {
                            return Err(format!("{qual}: match arm {cond:?} is not a variant of {ret}"));
                        }
                        w.line(&format!("case {}:", quote(cond)));
                    } else {
                        // No expression grammar yet: the arm is a Go boolean
                        // expression over `this`.
                        w.line(&format!("case {cond}:"));
                    }
                    self.transition(w, pid, c);
                }
                w.line("}");
                if !arms.iter().any(|(c, _)| c == "else") {
                    w.line(&format!("return zero, fmt.Errorf({}, this)", quote(&format!("{qual}: no match arm for %v"))));
                }
            }
            Edge::ForEach { dim, .. } => {
                let r = proc.graph.opens[&n];
                let region = &proc.graph.regions[r];
                let sink = region.sink.unwrap();
                let out = self.go_type(prog.ret(sink));
                w.line(&format!(
                    "results, err := fanOut(ctx, {}, func(ctx context.Context, elem {}) ({out}, error) {{",
                    self.reference(dim),
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

    fn transition(&mut self, w: &mut Writer, pid: NodeId, c: &Call) {
        let input = self.input(c);
        w.line(&format!("env.in{} = {input}", gname(self.prog, c.target)));
        w.line(&format!("pc = {}", pc_name(self.prog, pid, c.target)));
        w.line("continue");
    }
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

const RUNTIME: &str = r#"// fanOut runs f over xs concurrently and returns the results in input order.
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
