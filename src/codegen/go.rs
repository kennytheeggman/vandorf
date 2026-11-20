//! Go code generation.
//!
//! # Model
//!
//! Sequential edges are tail calls, so a proc compiles to a state machine
//! rather than nested calls: each node gets a program counter and an input and
//! output slot in a per-proc env struct, and an edge is "write the target's
//! input slot, set pc, continue". Back edges (`Loop -> Loop`) don't grow the
//! stack.
//!
//! A `for each` opens a region (see [`crate::sema::Region`]), compiled to its
//! own state machine function that runs once per element, concurrently, on a
//! copy of the env. The parent resumes at the collect target with the ordered
//! results bound to `this`.
//!
//! # Implementations
//!
//! A func's `= "file:function"` names the Go function to call, with the
//! signature
//!
//! ```go
//! func Function(ctx context.Context, <params in declared order>) (<ret>, error)
//! func Function(ctx context.Context, <params>) error // void funcs
//! ```
//!
//! The file path is relative to the generated package's directory. A file in
//! that directory is in the same package and is called directly; a file in
//! another directory is imported as `<import base>/<dir>` and called as
//! `<dir>.Function`. Funcs without an implementation call a generated stub
//! that returns an error.
//!
//! # Types
//!
//! Vandorf types are structural and map onto anonymous Go types, so impls
//! need no generated types and can live in any package: structs become
//! structs with exported fields in declaration order, enums become
//! `struct{ Tag string; <Variant> <payload>; ... }`. Where vandorf coerces (an
//! enum into a wider one, or a struct with reordered fields), a generated
//! `coerceN` helper converts.

use crate::sema::{Call, Edge, Expr, NodeId, NodeKind, Program, Ref, RegionId, Root};
use crate::types::{Prim, Type};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

#[derive(Debug, Clone)]
pub struct GoOptions {
    pub package: String,
    /// Import path of the generated package, used to import impls that live
    /// in other directories.
    pub import_base: Option<String>,
}

#[derive(Debug)]
pub struct GoOutput {
    /// File name -> source.
    pub files: BTreeMap<String, String>,
    /// Every implementation the generated code calls, for the caller to verify.
    pub impls: Vec<ImplUse>,
}

#[derive(Debug, Clone)]
pub struct ImplUse {
    pub node: String,
    pub file: String,
    pub function: String,
    /// The Go signature the implementation must have.
    pub signature: String,
}

pub fn generate(prog: &Program, opts: &GoOptions) -> Result<GoOutput, String> {
    let mut g = Gen {
        prog,
        opts,
        imports: BTreeMap::new(),
        coercions: HashMap::new(),
        helpers: vec![],
        stubs: BTreeSet::new(),
    };
    g.check_names()?;

    let mut files = BTreeMap::new();
    for &p in &prog.top_procs {
        g.imports.clear();
        let mut body = Writer::default();
        g.emit_proc_tree(&mut body, p)?;
        let mut imports: Vec<(String, Option<String>)> =
            vec![("context".into(), None), ("fmt".into(), None)];
        for (path, alias) in &g.imports {
            imports.push((path.clone(), Some(alias.clone())));
        }
        let mut w = header(&opts.package, &imports);
        w.raw(&body.out);
        files.insert(format!("{}_vandorf.go", prog.node(p).name().to_lowercase()), w.out);
    }

    let mut w = header(&opts.package, &[]);
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
    files.insert("types_vandorf.go".into(), w.out);

    if !g.stubs.is_empty() {
        let mut w = header(&opts.package, &[("context".into(), None), ("errors".into(), None)]);
        for &n in &g.stubs.clone() {
            let decl = prog.node(n);
            let stub = format!("unimplemented{}", gname(prog, n));
            w.line(&format!("// {stub} stands in for {}, which names no implementation. Write", decl.qual_name()));
            w.line(&format!("//\t{}", g.signature(n, "Name", true)));
            w.line("// and add `= \"file.go:Name\"` to its declaration.");
            w.line(&format!("{} {{", g.signature(n, &stub, false)));
            let ret = prog.ret(n);
            if ret.is_void() {
                w.line(&format!("return errors.New({})", go_quote(&format!("{} is not implemented", decl.qual_name()))));
            } else {
                w.line(&format!("var zero {}", g.go_type(ret)));
                w.line(&format!("return zero, errors.New({})", go_quote(&format!("{} is not implemented", decl.qual_name()))));
            }
            w.line("}");
            w.line("");
        }
        files.insert("stubs_vandorf.go".into(), w.out);
    }

    let mut w = header(&opts.package, &[("context".into(), None), ("sync".into(), None)]);
    w.raw(RUNTIME);
    for h in &g.helpers {
        w.line("");
        w.raw(h);
    }
    files.insert("runtime_vandorf.go".into(), w.out);

    let impls = prog
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(id, n)| match &n.kind {
            NodeKind::Func(f) => f.imp.as_ref().map(|i| ImplUse {
                node: n.qual_name(),
                file: i.file.clone(),
                function: i.function.clone(),
                signature: g.signature(id, &i.function, true),
            }),
            NodeKind::Proc(_) => None,
        })
        .collect();
    Ok(GoOutput { files, impls })
}

struct Gen<'a> {
    prog: &'a Program,
    opts: &'a GoOptions,
    /// Impl imports of the file being generated: path -> alias.
    imports: BTreeMap<String, String>,
    coercions: HashMap<(String, String), String>,
    helpers: Vec<String>,
    stubs: BTreeSet<NodeId>,
}

/// Go name of a node: its qualified name, segments exported and joined.
fn gname(prog: &Program, n: NodeId) -> String {
    prog.node(n).qual.iter().map(|s| exported(s)).collect()
}

impl Gen<'_> {
    // ---- Names ----

    /// Rejects programs whose names would collide or be invalid once mapped
    /// to Go.
    fn check_names(&self) -> Result<(), String> {
        let mut seen: HashMap<String, String> = HashMap::new();
        for (id, n) in self.prog.nodes.iter().enumerate() {
            let g = gname(self.prog, id);
            if let Some(prev) = seen.insert(g.clone(), n.qual_name()) {
                return Err(format!("{prev} and {} both map to the Go name {g}", n.qual_name()));
            }
            let mut types: Vec<&Type> = vec![self.prog.ret(id)];
            if let NodeKind::Func(f) = &n.kind {
                types.extend(f.params.iter().map(|(_, t)| t));
                check_fields(&f.params.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), false)
                    .map_err(|e| format!("{}: {e}", n.qual_name()))?;
            }
            for t in types {
                check_type(t).map_err(|e| format!("{}: {e}", n.qual_name()))?;
            }
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
        let Some(imp) = &f.imp else {
            self.stubs.insert(n);
            return Ok(format!("unimplemented{}", gname(self.prog, n)));
        };
        let dir = match imp.file.rsplit_once('/') {
            Some((d, _)) => d.trim_end_matches('/'),
            None => "",
        };
        if dir.is_empty() || dir == "." {
            return Ok(imp.function.clone());
        }
        let Some(base) = &self.opts.import_base else {
            return Err(format!(
                "{} is implemented in {}, outside the generated package, but no import base is known (pass --module or run inside a Go module)",
                self.prog.node(n).qual_name(), imp.file,
            ));
        };
        let path = join_import(base, dir)
            .ok_or_else(|| format!("{}: cannot resolve import for {}", self.prog.node(n).qual_name(), imp.file))?;
        let alias = match self.imports.get(&path) {
            Some(a) => a.clone(),
            None => {
                let stem: String = path.rsplit('/').next().unwrap().chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect();
                let mut alias = stem.clone();
                let mut i = 2;
                while self.imports.values().any(|a| *a == alias) || ["context", "fmt"].contains(&alias.as_str()) {
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

    fn go_type(&self, t: &Type) -> String {
        match t {
            Type::Prim(p) => match p {
                Prim::Void => "struct{}",
                Prim::Int => "int",
                Prim::Float => "float64",
                Prim::Bool => "bool",
                Prim::String => "string",
            }
            .into(),
            Type::Array(t) => format!("[]{}", self.go_type(t)),
            Type::Struct(fs) => {
                let fs: Vec<String> = fs.iter().map(|(n, t)| format!("{} {}", exported(n), self.go_type(t))).collect();
                format!("struct{{ {} }}", fs.join("; "))
            }
            Type::Enum(vs) => {
                let mut fs = vec!["Tag string".to_string()];
                fs.extend(vs.iter().map(|(n, t)| format!("{} {}", exported(n), self.go_type(t))));
                format!("struct{{ {} }}", fs.join("; "))
            }
        }
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
        let mut w = Writer::default();
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
                    w.line(&format!("case {}:", go_quote(n)));
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
                    format!("{}{{Tag: {}, {}: {p}}}", self.go_type(&a.param), go_quote(tag), exported(tag))
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
        let sig: Vec<String> = params.iter().map(|(p, t)| format!("{} {}", go_param(p), self.go_type(t))).collect();
        w.line(&format!("// Run{name} runs proc {}.", prog.node(pid).qual_name()));
        w.line(&format!("func Run{name}(ctx context.Context{}) ({ret}, error) {{", sig.iter().map(|s| format!(", {s}")).collect::<String>()));
        w.line(&format!("var zero {ret}"));
        w.line(&format!("env := &{env}{{}}"));
        let fields: Vec<String> = params.iter().map(|(p, _)| format!("{}: {}", exported(p), go_param(p))).collect();
        w.line(&format!("env.in{} = {}{{{}}}", gname(prog, proc.entry), self.in_type(proc.entry), fields.join(", ")));
        w.line(&format!("pc := {}", pc_name(prog, pid, proc.entry)));
        self.emit_loop(w, pid, 0)?;
        w.line("}");
        w.line("");

        for (&opener, &r) in sorted(&g.opens) {
            let region = &g.regions[r];
            let Some(Edge::ForEach { call, .. }) = g.edges.get(&opener) else { unreachable!() };
            let sink = self.go_type(prog.ret(region.sink.unwrap()));
            w.line(&format!("// {} runs one element of the for each from {}.", region_fn(prog, pid, r), prog.node(opener).qual_name()));
            w.line(&format!(
                "func {}(ctx context.Context, env *{env}, elem {}) ({sink}, error) {{",
                region_fn(prog, pid, r), self.go_type(&region.elem),
            ));
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
        w.line(&format!("return zero, fmt.Errorf({}, pc)", go_quote(&format!("{}: invalid state %d", self.prog.node(pid).qual_name()))));
        w.line("}");
        w.line("}");
        Ok(())
    }

    fn emit_node(&mut self, w: &mut Writer, pid: NodeId, n: NodeId) -> Result<(), String> {
        let prog = self.prog;
        let proc = prog.proc(pid);
        let id = gname(prog, n);
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
        w.line(&format!("return zero, fmt.Errorf({}, err)", go_quote(&format!("{}: %w", prog.node(n).qual_name()))));
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
                            return Err(format!("{}: match arm {cond:?} is not a variant of {ret}", prog.node(n).qual_name()));
                        }
                        w.line(&format!("case {}:", go_quote(cond)));
                    } else {
                        // No expression grammar yet: the arm is a Go boolean
                        // expression over `this`.
                        w.line(&format!("case {cond}:"));
                    }
                    self.transition(w, pid, c);
                }
                w.line("}");
                if !arms.iter().any(|(c, _)| c == "else") {
                    w.line(&format!("return zero, fmt.Errorf({}, this)", go_quote(&format!("{}: no match arm for %v", prog.node(n).qual_name()))));
                }
            }
            Edge::ForEach { dim, .. } => {
                let r = proc.graph.opens[&n];
                let region = &proc.graph.regions[r];
                let sink = region.sink.unwrap();
                let out = self.go_type(prog.ret(sink));
                w.line(&format!(
                    "results, err := fanOut(ctx, {}, func(ctx context.Context, elem {}) ({out}, error) {{",
                    self.reference(dim), self.go_type(&region.elem),
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

fn sorted<K: Ord, V>(m: &HashMap<K, V>) -> Vec<(&K, &V)> {
    let mut v: Vec<_> = m.iter().collect();
    v.sort_by(|a, b| a.0.cmp(b.0));
    v
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

fn check_type(t: &Type) -> Result<(), String> {
    match t {
        Type::Prim(_) => Ok(()),
        Type::Array(t) => check_type(t),
        Type::Struct(fs) | Type::Enum(fs) => {
            check_fields(&fs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), matches!(t, Type::Enum(_)))?;
            fs.iter().try_for_each(|(_, t)| check_type(t))
        }
    }
}

/// Joins a relative directory onto an import path, resolving `.` and `..`.
fn join_import(base: &str, dir: &str) -> Option<String> {
    let mut segs: Vec<&str> = base.split('/').collect();
    for s in dir.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                segs.pop()?;
            }
            s => segs.push(s),
        }
    }
    (!segs.is_empty()).then(|| segs.join("/"))
}

fn exported(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
        None => String::new(),
    }
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

fn go_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn header(pkg: &str, imports: &[(String, Option<String>)]) -> Writer {
    let mut w = Writer::default();
    w.line("// Code generated by vandorf. DO NOT EDIT.");
    w.line("");
    w.line(&format!("package {pkg}"));
    w.line("");
    if !imports.is_empty() {
        w.line("import (");
        for (path, alias) in imports {
            match alias {
                Some(a) => w.line(&format!("{a} {}", go_quote(path))),
                None => w.line(&go_quote(path)),
            }
        }
        w.line(")");
        w.line("");
    }
    w
}

/// Emits lines with gofmt-style tab indentation inferred from braces, so the
/// output is readable without running gofmt.
#[derive(Default)]
struct Writer {
    out: String,
    depth: usize,
}

impl Writer {
    fn line(&mut self, s: &str) {
        let s = s.trim();
        if s.starts_with('}') || s.starts_with(')') {
            self.depth = self.depth.saturating_sub(1);
        }
        let depth = if s.starts_with("case ") || s == "default:" { self.depth.saturating_sub(1) } else { self.depth };
        if !s.is_empty() {
            self.out.push_str(&"\t".repeat(depth));
        }
        self.out.push_str(s);
        self.out.push('\n');
        if s.ends_with('{') || s.ends_with('(') {
            self.depth += 1;
        }
    }

    fn raw(&mut self, s: &str) {
        self.out.push_str(s);
    }
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
