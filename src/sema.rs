//! Name resolution, type checking and for-each region analysis.
//!
//! Turns the [`ast`] into a [`Program`]: every func and proc declaration gets a
//! [`NodeId`], and every proc gets a [`Graph`] whose edges point at node ids
//! and whose argument expressions are typed. Both the document generator and
//! the code generator work from this.

use crate::ast::{self, Span};
use crate::types::{Prim, Type, TypeDef, TypeId};
use std::collections::{BTreeMap, HashMap, HashSet};

pub type NodeId = usize;
pub type RegionId = usize;

#[derive(Debug, Clone)]
pub struct Error {
    pub msg: String,
    pub span: Span,
}

fn err<T>(span: Span, msg: impl Into<String>) -> Result<T, Error> {
    Err(Error { msg: msg.into(), span })
}

#[derive(Debug)]
pub struct Program {
    /// `type` declarations; [`Type::Named`] indexes into this.
    pub types: Vec<TypeDef>,
    pub nodes: Vec<NodeDecl>,
    pub globals: BTreeMap<String, Global>,
    /// Top-level procs in source order.
    pub top_procs: Vec<NodeId>,
}

#[derive(Debug)]
pub enum Global {
    Interface(BTreeMap<String, NodeId>),
    Node(NodeId),
}

#[derive(Debug)]
pub struct NodeDecl {
    /// Qualified name: `["Foo", "Call"]` for an interface func, `["Bar", "Start"]`
    /// for a func local to proc `Bar`, `["Bar"]` for a top-level proc.
    pub qual: Vec<String>,
    pub span: Span,
    pub kind: NodeKind,
}

impl NodeDecl {
    pub fn name(&self) -> &str {
        self.qual.last().unwrap()
    }
    pub fn qual_name(&self) -> String {
        self.qual.join(".")
    }
}

#[derive(Debug)]
pub enum NodeKind {
    Func(FuncDecl),
    Proc(ProcDecl),
    /// `-> B(..) as B2`: a separate instance of node `B`, local to the proc
    /// the edge is in. It has `B`'s parameters, return and implementation,
    /// but its own output and outgoing edge.
    Alias(NodeId),
}

#[derive(Debug)]
pub struct FuncDecl {
    pub params: Vec<(String, Type)>,
    pub ret: Type,
    pub imps: Vec<ast::ImplRef>,
}

impl FuncDecl {
    /// The implementation reference whose file has the given extension.
    pub fn imp(&self, ext: &str) -> Option<&ast::ImplRef> {
        self.imps.iter().find(|i| i.file.rsplit_once('.').is_some_and(|(_, e)| e == ext))
    }
}

#[derive(Debug)]
pub struct ProcDecl {
    pub parent: Option<NodeId>,
    pub entry: NodeId,
    pub ret: Type,
    /// Locally declared nodes, in source order.
    pub locals: Vec<NodeId>,
    pub graph: Graph,
}

#[derive(Debug, Default)]
pub struct Graph {
    /// Outgoing edge of each source node (at most one per node).
    pub edges: HashMap<NodeId, Edge>,
    /// `regions[0]` is the proc body; the rest are for-each regions.
    pub regions: Vec<Region>,
    /// Region of every node reachable from the entry.
    pub region_of: HashMap<NodeId, RegionId>,
    /// For-each source node -> the region it opens.
    pub opens: HashMap<NodeId, RegionId>,
}

/// One state machine in the generated code: the proc body, or the part of
/// the graph run once per element of a `for each`, up to its `collect`.
#[derive(Debug)]
pub struct Region {
    pub parent: Option<RegionId>,
    pub entry: NodeId,
    /// Type of `this` on entry (the element type for a for-each region).
    pub elem: Type,
    /// Nodes in breadth-first order from the entry.
    pub nodes: Vec<NodeId>,
    /// The node whose `collect` edge closes the region.
    pub sink: Option<NodeId>,
}

#[derive(Debug, Clone)]
pub enum Edge {
    Call(Call),
    Match(Vec<(String, Call)>),
    ForEach { dim: Ref, call: Call },
    Collect(Call),
}

impl Edge {
    pub fn calls(&self) -> Vec<&Call> {
        match self {
            Edge::Call(c) | Edge::Collect(c) | Edge::ForEach { call: c, .. } => vec![c],
            Edge::Match(arms) => arms.iter().map(|(_, c)| c).collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Call {
    pub target: NodeId,
    /// Arguments in the target's parameter order.
    pub args: Vec<CallArg>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct CallArg {
    pub name: String,
    pub param: Type,
    pub value: Expr,
}

/// A value, checked against the type it's passed as (a parameter, or a
/// field, element or payload of one); backends render it at that type.
#[derive(Debug, Clone)]
pub enum Expr {
    Ref(Ref),
    Int(i64),
    /// Also an int literal passed as a float.
    Float(f64),
    Str(String),
    Bool(bool),
    /// `Tag(payload)`; no payload for variants that carry void.
    Variant { tag: String, payload: Option<Box<Expr>> },
    /// All of the struct's fields, in the type's order.
    Struct(Vec<(String, Expr)>),
    Array(Vec<Expr>),
}

impl Expr {
    /// The node outputs this value reads.
    pub fn refs<'a>(&'a self, out: &mut Vec<&'a Ref>) {
        match self {
            Expr::Ref(r) => out.push(r),
            Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) => {}
            Expr::Variant { payload, .. } => payload.iter().for_each(|p| p.refs(out)),
            Expr::Struct(fs) => fs.iter().for_each(|(_, e)| e.refs(out)),
            Expr::Array(es) => es.iter().for_each(|e| e.refs(out)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Ref {
    pub root: Root,
    /// Type of the root (`this`, or the node's output).
    pub root_ty: Type,
    pub fields: Vec<String>,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    This,
    /// The output of a node that already ran.
    Node(NodeId),
}

impl Program {
    pub fn node(&self, id: NodeId) -> &NodeDecl {
        &self.nodes[id]
    }

    pub fn proc(&self, id: NodeId) -> &ProcDecl {
        match &self.nodes[id].kind {
            NodeKind::Proc(p) => p,
            _ => panic!("{} is not a proc", self.nodes[id].qual_name()),
        }
    }

    /// Parameters of a node. A proc takes its entry's parameters.
    pub fn params(&self, id: NodeId) -> &[(String, Type)] {
        match &self.nodes[id].kind {
            NodeKind::Func(f) => &f.params,
            NodeKind::Proc(p) => self.params(p.entry),
            NodeKind::Alias(b) => self.params(*b),
        }
    }

    pub fn ret(&self, id: NodeId) -> &Type {
        match &self.nodes[id].kind {
            NodeKind::Func(f) => &f.ret,
            NodeKind::Proc(p) => &p.ret,
            NodeKind::Alias(b) => self.ret(*b),
        }
    }

    /// The func or proc a node runs: itself, or what an alias instances.
    pub fn base(&self, id: NodeId) -> NodeId {
        match self.nodes[id].kind {
            NodeKind::Alias(b) => b,
            _ => id,
        }
    }

    /// All procs, nested ones included, parents before children.
    pub fn all_procs(&self) -> Vec<NodeId> {
        let mut out = vec![];
        let mut stack: Vec<NodeId> = self.top_procs.iter().rev().copied().collect();
        while let Some(p) = stack.pop() {
            out.push(p);
            for &l in self.proc(p).locals.iter().rev() {
                if matches!(self.nodes[l].kind, NodeKind::Proc(_)) {
                    stack.push(l);
                }
            }
        }
        out
    }
}

pub fn analyze(file: &ast::File) -> Result<Program, Vec<Error>> {
    let mut cx = Cx {
        prog: Program { types: vec![], nodes: vec![], globals: BTreeMap::new(), top_procs: vec![] },
        type_names: HashMap::new(),
        procs: vec![],
        aliases: vec![],
        errors: vec![],
    };
    cx.declare_types(&file.types);
    if !cx.errors.is_empty() {
        return Err(cx.errors);
    }
    cx.declare_file(file);
    cx.resolve_aliases();
    if !cx.errors.is_empty() {
        return Err(cx.errors);
    }
    for (pid, p) in std::mem::take(&mut cx.procs) {
        if let Err(e) = cx.build_graph(pid, p) {
            cx.errors.push(e);
        }
    }
    if cx.errors.is_empty() { Ok(cx.prog) } else { Err(cx.errors) }
}

struct Cx<'a> {
    prog: Program,
    type_names: HashMap<String, TypeId>,
    procs: Vec<(NodeId, &'a ast::Proc)>,
    /// Alias nodes, the proc they're in, and the target declaring them.
    aliases: Vec<(NodeId, NodeId, &'a ast::Target)>,
    errors: Vec<Error>,
}

/// The targets of an edge.
fn targets(k: &ast::EdgeKind) -> Vec<&ast::Target> {
    match k {
        ast::EdgeKind::Call(t) | ast::EdgeKind::Collect { target: t, .. } | ast::EdgeKind::ForEach { target: t, .. } => vec![t],
        ast::EdgeKind::Match(arms) => arms.iter().map(|a| &a.target).collect(),
    }
}

impl<'a> Cx<'a> {
    // ---- Types ----

    /// Declares all typedefs before resolving any, so they can refer to each
    /// other in any order, then rejects cycles that never reach a structure.
    fn declare_types(&mut self, decls: &[ast::TypeDecl]) {
        for d in decls {
            let name = &d.name.name;
            if Prim::from_name(name).is_some() {
                self.errors.push(Error { msg: format!("{name} is a built-in type"), span: d.name.span });
            } else if self.type_names.contains_key(name) {
                self.errors.push(Error { msg: format!("duplicate type {name}"), span: d.name.span });
            } else {
                self.type_names.insert(name.clone(), self.prog.types.len());
                self.prog.types.push(TypeDef { name: name.clone(), ty: Type::Prim(Prim::Void), recursive: false });
            }
        }
        let mut ok = vec![];
        for d in decls {
            let Some(&id) = self.type_names.get(&d.name.name) else { continue };
            if ok.contains(&id) {
                continue; // a duplicate's second declaration
            }
            if let Some(t) = self.ty(&d.ty) {
                self.prog.types[id].ty = t;
                ok.push(id);
            }
        }
        if !self.errors.is_empty() {
            return;
        }
        // Typedef reference graph; a typedef is recursive if it reaches itself.
        let edges: Vec<Vec<TypeId>> = self.prog.types.iter().map(|d| {
            let mut out = vec![];
            d.ty.named(&mut out);
            out
        }).collect();
        for (id, d) in decls.iter().enumerate() {
            let mut seen = vec![false; edges.len()];
            let mut stack = edges[id].clone();
            while let Some(n) = stack.pop() {
                if !std::mem::replace(&mut seen[n], true) {
                    stack.extend(&edges[n]);
                }
            }
            self.prog.types[id].recursive = seen[id];
            let def = &self.prog.types[id];
            if def.recursive && !matches!(def.ty, Type::Struct(_) | Type::Enum(_)) {
                self.errors.push(Error {
                    msg: format!("type {} refers to itself, so it must be a struct or enum", d.name.name),
                    span: d.name.span,
                });
            } else if !def.recursive && def.ty.unfold(&self.prog.types).is_void() {
                self.errors.push(Error { msg: format!("type {} names void", d.name.name), span: d.name.span });
            }
        }
    }

    // ---- Declarations ----

    fn declare_file(&mut self, file: &'a ast::File) {
        for i in &file.interfaces {
            let mut funcs = BTreeMap::new();
            for f in &i.funcs {
                if funcs.contains_key(&f.name.name) {
                    self.errors.push(Error {
                        msg: format!("duplicate func {} in interface {}", f.name.name, i.name.name),
                        span: f.name.span,
                    });
                    continue;
                }
                if let Some(id) = self.declare_func(f, vec![i.name.name.clone()]) {
                    funcs.insert(f.name.name.clone(), id);
                }
            }
            self.declare_global(&i.name, Global::Interface(funcs));
        }
        for n in &file.nodes {
            let (name, id) = match n {
                ast::Node::Func(f) => (&f.name, self.declare_func(f, vec![])),
                ast::Node::Proc(p) => (&p.name, self.declare_proc(p, vec![], None)),
            };
            if let Some(id) = id {
                if let ast::Node::Proc(_) = n {
                    self.prog.top_procs.push(id);
                }
                self.declare_global(name, Global::Node(id));
            }
        }
    }

    fn declare_global(&mut self, name: &ast::Ident, g: Global) {
        if self.prog.globals.contains_key(&name.name) {
            self.errors.push(Error { msg: format!("duplicate name {}", name.name), span: name.span });
        } else {
            self.prog.globals.insert(name.name.clone(), g);
        }
    }

    fn push_node(&mut self, decl: NodeDecl) -> NodeId {
        self.prog.nodes.push(decl);
        self.prog.nodes.len() - 1
    }

    fn declare_func(&mut self, f: &ast::Func, mut qual: Vec<String>) -> Option<NodeId> {
        qual.push(f.name.name.clone());
        let params = self.fields(&f.params);
        let ret = match &f.ret {
            Some(t) => self.ty(t),
            None => Some(Type::Prim(Prim::Void)),
        };
        let decl = FuncDecl { params: params?, ret: ret?, imps: f.imps.clone() };
        Some(self.push_node(NodeDecl { qual, span: f.name.span, kind: NodeKind::Func(decl) }))
    }

    fn declare_proc(
        &mut self,
        p: &'a ast::Proc,
        mut qual: Vec<String>,
        parent: Option<NodeId>,
    ) -> Option<NodeId> {
        qual.push(p.name.name.clone());
        let ret = self.ty(&p.ret)?;
        // Reserve the id first so locals can name their parent.
        let id = self.push_node(NodeDecl {
            qual: qual.clone(),
            span: p.name.span,
            kind: NodeKind::Proc(ProcDecl {
                parent,
                entry: usize::MAX,
                ret,
                locals: vec![],
                graph: Graph::default(),
            }),
        });
        let mut locals = vec![];
        let mut names: HashMap<&str, NodeId> = HashMap::new();
        // Alias name -> the node path it instances.
        let mut aliased: HashMap<&str, String> = HashMap::new();
        for n in &p.nodes {
            let (name, lid) = match n {
                ast::Node::Func(f) => (&f.name, self.declare_func(f, qual.clone())),
                ast::Node::Proc(sub) => (&sub.name, self.declare_proc(sub, qual.clone(), Some(id))),
            };
            let Some(lid) = lid else { continue };
            if names.insert(&name.name, lid).is_some() {
                self.errors.push(Error {
                    msg: format!("duplicate name {} in proc {}", name.name, p.name.name),
                    span: name.span,
                });
            }
            locals.push(lid);
        }
        for t in p.edges.iter().flat_map(|e| targets(&e.kind)) {
            let Some(a) = &t.alias else { continue };
            let of = t.node.to_string();
            match (names.contains_key(a.name.as_str()), aliased.get(a.name.as_str())) {
                // Another call into the same instance.
                (_, Some(prev)) if *prev == of => continue,
                (_, Some(prev)) => {
                    self.errors.push(Error { msg: format!("{} is already an alias of {prev}", a.name), span: a.span });
                    continue;
                }
                (true, None) => {
                    self.errors.push(Error {
                        msg: format!("alias {} clashes with a node of that name in proc {}", a.name, p.name.name),
                        span: a.span,
                    });
                    continue;
                }
                (false, None) => {}
            }
            let mut q = qual.clone();
            q.push(a.name.clone());
            let lid = self.push_node(NodeDecl { qual: q, span: a.span, kind: NodeKind::Alias(usize::MAX) });
            names.insert(&a.name, lid);
            aliased.insert(&a.name, of);
            locals.push(lid);
            self.aliases.push((lid, id, t));
        }
        let Some(&entry) = names.get(p.entry.name.as_str()) else {
            self.errors.push(Error {
                msg: format!("entry {} is not declared in proc {}", p.entry.name, p.name.name),
                span: p.entry.span,
            });
            return None;
        };
        let NodeKind::Proc(decl) = &mut self.prog.nodes[id].kind else { unreachable!() };
        decl.entry = entry;
        decl.locals = locals;
        self.procs.push((id, p));
        Some(id)
    }

    /// Points each alias at the node it instances. Runs once every node is
    /// declared, since the target may be a global declared later.
    fn resolve_aliases(&mut self) {
        for (lid, pid, t) in std::mem::take(&mut self.aliases) {
            match self.resolve_node(pid, &t.node) {
                Ok(b) if matches!(self.prog.nodes[b].kind, NodeKind::Alias(_)) => self.errors.push(Error {
                    msg: format!("{} is itself an alias; alias the node it instances", t.node),
                    span: t.node.span,
                }),
                Ok(b) => self.prog.nodes[lid].kind = NodeKind::Alias(b),
                Err(e) => self.errors.push(e),
            }
        }
    }

    fn fields(&mut self, fields: &[ast::Field]) -> Option<Vec<(String, Type)>> {
        let mut out: Vec<(String, Type)> = vec![];
        let mut ok = true;
        for f in fields {
            if out.iter().any(|(n, _)| *n == f.name.name) {
                self.errors.push(Error { msg: format!("duplicate field {}", f.name.name), span: f.name.span });
                ok = false;
            }
            match self.ty(&f.ty) {
                Some(t) => out.push((f.name.name.clone(), t)),
                None => ok = false,
            }
        }
        ok.then_some(out)
    }

    fn ty(&mut self, t: &ast::TypeExpr) -> Option<Type> {
        use ast::TypeExprKind as K;
        Some(match &t.kind {
            K::Named(n) => match Prim::from_name(&n.name) {
                Some(p) => Type::Prim(p),
                None if self.type_names.contains_key(&n.name) => Type::Named(self.type_names[&n.name], n.name.clone()),
                None => {
                    self.errors.push(Error { msg: format!("unknown type {}", n.name), span: n.span });
                    return None;
                }
            },
            K::Array(t) => Type::Array(Box::new(self.ty(t)?)),
            K::Struct(fs) => Type::Struct(self.fields(fs)?),
            K::Enum(vs) => {
                let mut out: Vec<(String, Type)> = vec![];
                for v in vs {
                    if out.iter().any(|(n, _)| *n == v.name.name) {
                        self.errors.push(Error { msg: format!("duplicate variant {}", v.name.name), span: v.name.span });
                        return None;
                    }
                    let t = match &v.payload {
                        Some(p) => self.ty(p)?,
                        None => Type::Prim(Prim::Void),
                    };
                    out.push((v.name.name.clone(), t));
                }
                Type::Enum(out)
            }
        })
    }

    // ---- Graphs ----

    /// Resolves the node a path starts at, returning the remaining segments
    /// (field selections). Lookup goes through the enclosing procs' locals,
    /// innermost first, then globals; `Iface.Func` names an interface func.
    fn resolve<'p>(&self, pid: NodeId, path: &'p ast::Path) -> Result<(NodeId, &'p [ast::Ident]), Error> {
        let first = &path.segs[0];
        let mut scope = Some(pid);
        while let Some(p) = scope {
            let decl = self.prog.proc(p);
            if let Some(&n) = decl.locals.iter().find(|&&l| self.prog.nodes[l].name() == first.name) {
                return Ok((n, &path.segs[1..]));
            }
            scope = decl.parent;
        }
        match self.prog.globals.get(&first.name) {
            Some(Global::Node(n)) => Ok((*n, &path.segs[1..])),
            Some(Global::Interface(funcs)) => {
                let Some(second) = path.segs.get(1) else {
                    return err(first.span, format!("{} is an interface, not a node", first.name));
                };
                match funcs.get(&second.name) {
                    Some(&n) => Ok((n, &path.segs[2..])),
                    None => err(second.span, format!("interface {} has no func {}", first.name, second.name)),
                }
            }
            None => err(first.span, format!("unknown node {}", first.name)),
        }
    }

    fn resolve_node(&self, pid: NodeId, path: &ast::Path) -> Result<NodeId, Error> {
        let (n, rest) = self.resolve(pid, path)?;
        if let Some(r) = rest.first() {
            return err(r.span, format!("{} is a node; expected a node name, not a field selection", self.prog.nodes[n].qual_name()));
        }
        Ok(n)
    }

    fn reference(&self, pid: NodeId, path: &ast::Path, this: &Type) -> Result<Ref, Error> {
        let (root, mut ty, rest) = if path.segs[0].name == "this" {
            (Root::This, this.clone(), &path.segs[1..])
        } else {
            let (n, rest) = self.resolve(pid, path)?;
            (Root::Node(n), self.prog.ret(n).clone(), rest)
        };
        let root_ty = ty.clone();
        let mut fields = vec![];
        for f in rest {
            ty = match ty.field(&f.name, &self.prog.types) {
                Some(t) => t.clone(),
                None => return err(f.span, format!("{ty} has no field {}", f.name)),
            };
            fields.push(f.name.clone());
        }
        Ok(Ref { root, root_ty, fields, ty, span: path.span })
    }

    /// Checks `v` against the type it's passed as.
    fn value(&self, pid: NodeId, v: &ast::Value, want: &Type, this: &Type) -> Result<Expr, Error> {
        use ast::ValueKind as V;
        let defs = &self.prog.types;
        let shape = want.unfold(defs);
        let mismatch = |found: &str| err(v.span, format!("expected {want}, found {found}"));
        Ok(match &v.kind {
            V::Ref(p) => {
                let r = self.reference(pid, p, this)?;
                if !want.accepts(&r.ty, defs) {
                    return err(p.span, format!("{p} has type {}, but {want} is expected", r.ty));
                }
                Expr::Ref(r)
            }
            V::Int(i) => match shape {
                Type::Prim(Prim::Int) => Expr::Int(*i),
                Type::Prim(Prim::Float) => Expr::Float(*i as f64),
                _ => return mismatch("an int literal"),
            },
            V::Float(f) => match shape {
                Type::Prim(Prim::Float) => Expr::Float(*f),
                _ => return mismatch("a float literal"),
            },
            V::Str(s) => match shape {
                Type::Prim(Prim::String) => Expr::Str(s.clone()),
                _ => return mismatch("a string literal"),
            },
            V::Bool(b) => match shape {
                Type::Prim(Prim::Bool) => Expr::Bool(*b),
                _ => return mismatch("a bool literal"),
            },
            V::Array(items) => {
                let Type::Array(elem) = shape else { return mismatch("an array literal") };
                Expr::Array(items.iter().map(|i| self.value(pid, i, elem, this)).collect::<Result<_, _>>()?)
            }
            V::Struct(args) => {
                let Type::Struct(fs) = shape else { return mismatch("a struct literal") };
                let mut given: HashMap<&str, Expr> = HashMap::new();
                for a in args {
                    let Some((_, ft)) = fs.iter().find(|(n, _)| *n == a.name.name) else {
                        return err(a.name.span, format!("{want} has no field {}", a.name.name));
                    };
                    let e = self.value(pid, &a.value, ft, this)?;
                    if given.insert(&a.name.name, e).is_some() {
                        return err(a.name.span, format!("field {} given twice", a.name.name));
                    }
                }
                let mut out = vec![];
                for (n, _) in fs {
                    match given.remove(n.as_str()) {
                        Some(e) => out.push((n.clone(), e)),
                        None => return err(v.span, format!("struct literal is missing field {n} of {want}")),
                    }
                }
                Expr::Struct(out)
            }
            V::Variant { tag, payload } => {
                let Type::Enum(_) = shape else { return mismatch(&format!("variant {}", tag.name)) };
                let Some(vty) = want.variant(&tag.name, defs) else {
                    return err(tag.span, format!("{want} has no variant {}", tag.name));
                };
                let payload = match payload {
                    None if vty.is_void() => None,
                    None => return err(v.span, format!("variant {} carries {vty}; write {}(value)", tag.name, tag.name)),
                    // `S(this)` with a void `this` is allowed, and carries nothing.
                    Some(p) => {
                        let e = self.value(pid, p, vty, this)?;
                        (!vty.is_void()).then(|| Box::new(e))
                    }
                };
                Expr::Variant { tag: tag.name.clone(), payload }
            }
        })
    }

    /// The node a target calls: the alias instance for `-> B(..) as B2`.
    fn target(&self, pid: NodeId, t: &ast::Target) -> Result<NodeId, Error> {
        match &t.alias {
            Some(a) => Ok(self.prog.proc(pid).locals.iter().copied().find(|&l| self.prog.nodes[l].name() == a.name).unwrap()),
            None => self.resolve_node(pid, &t.node),
        }
    }

    fn call(&self, pid: NodeId, t: &ast::Target, this: &Type) -> Result<Call, Error> {
        let target = self.target(pid, t)?;
        let tname = self.prog.nodes[target].qual_name();
        let params = self.prog.params(target);
        let mut given: HashMap<&str, (&ast::Arg, Expr)> = HashMap::new();
        for a in &t.args {
            let Some((_, pty)) = params.iter().find(|(n, _)| *n == a.name.name) else {
                return err(a.name.span, format!("{tname} has no parameter {}", a.name.name));
            };
            let value = self.value(pid, &a.value, pty, this).map_err(|mut e| {
                e.msg = format!("argument {} of {tname}: {}", a.name.name, e.msg);
                e
            })?;
            if given.insert(&a.name.name, (a, value)).is_some() {
                return err(a.name.span, format!("argument {} given twice", a.name.name));
            }
        }
        let mut args = vec![];
        for (name, pty) in params {
            match given.remove(name.as_str()) {
                Some((_, value)) => args.push(CallArg { name: name.clone(), param: pty.clone(), value }),
                None => return err(t.node.span, format!("call to {tname} is missing argument {name}")),
            }
        }
        Ok(Call { target, args, span: t.node.span })
    }

    fn build_graph(&mut self, pid: NodeId, p: &ast::Proc) -> Result<(), Error> {
        let mut g = Graph::default();
        for e in &p.edges {
            let src = self.resolve_node(pid, &e.source)?;
            let ret = self.prog.ret(src).clone();
            let edge = match &e.kind {
                ast::EdgeKind::Call(t) => Edge::Call(self.call(pid, t, &ret)?),
                ast::EdgeKind::Match(arms) => Edge::Match(
                    arms.iter()
                        .map(|a| Ok((a.cond.clone(), self.call(pid, &a.target, &ret)?)))
                        .collect::<Result<_, Error>>()?,
                ),
                ast::EdgeKind::Collect { target, .. } => {
                    Edge::Collect(self.call(pid, target, &Type::Array(Box::new(ret)))?)
                }
                ast::EdgeKind::ForEach { dim, target } => {
                    let dim = self.reference(pid, dim, &ret)?;
                    let Type::Array(elem) = dim.ty.unfold(&self.prog.types) else {
                        return err(dim.span, format!("for each needs an array, found {}", dim.ty));
                    };
                    let call = self.call(pid, target, elem)?;
                    Edge::ForEach { dim, call }
                }
            };
            if g.edges.insert(src, edge).is_some() {
                return err(e.source.span, format!("{} already has an outgoing edge", e.source));
            }
        }
        let entry = self.prog.proc(pid).entry;
        let root = new_region(&mut g, None, entry, Type::Prim(Prim::Void));
        self.fill(&mut g, root, p)?;
        self.check_scopes(pid, &g, p)?;
        let NodeKind::Proc(decl) = &mut self.prog.nodes[pid].kind else { unreachable!() };
        decl.graph = g;
        Ok(())
    }

    /// Assigns every node reachable from a region's entry to that region,
    /// opening a child region at each `for each` and resuming after its
    /// `collect`.
    fn fill(&self, g: &mut Graph, r: RegionId, p: &ast::Proc) -> Result<(), Error> {
        let name = |n: NodeId| self.prog.nodes[n].qual_name();
        let mut work = vec![g.regions[r].entry];
        let mut i = 0;
        while i < work.len() {
            let n = work[i];
            i += 1;
            if let Some(&other) = g.region_of.get(&n) {
                if other != r {
                    return err(p.name.span, format!("{} is reachable both inside and outside a for each region", name(n)));
                }
                continue;
            }
            g.region_of.insert(n, r);
            g.regions[r].nodes.push(n);
            let Some(edge) = g.edges.get(&n).cloned() else {
                if r != 0 {
                    return err(p.name.span, format!("path through {} ends inside a for each region without a collect", name(n)));
                }
                continue;
            };
            match &edge {
                Edge::Call(c) => work.push(c.target),
                Edge::Match(arms) => work.extend(arms.iter().map(|(_, c)| c.target)),
                Edge::Collect(c) => {
                    if r == 0 {
                        return err(c.span, format!("{} collects outside of any for each", name(n)));
                    }
                    if let Some(s) = g.regions[r].sink.filter(|&s| s != n) {
                        return err(c.span, format!("for each region collects at both {} and {}", name(s), name(n)));
                    }
                    g.regions[r].sink = Some(n);
                }
                Edge::ForEach { dim, call } => {
                    let Type::Array(elem) = dim.ty.unfold(&self.prog.types) else { unreachable!() };
                    let child = new_region(g, Some(r), call.target, (**elem).clone());
                    g.opens.insert(n, child);
                    self.fill(g, child, p)?;
                    let Some(sink) = g.regions[child].sink else {
                        return err(call.span, format!("for each from {} never collects", name(n)));
                    };
                    let Some(Edge::Collect(c)) = g.edges.get(&sink) else { unreachable!() };
                    work.push(c.target);
                }
            }
        }
        Ok(())
    }

    /// Checks that arguments only read nodes whose output is in scope (their
    /// own region or an enclosing one), and that terminal nodes return the
    /// proc's type.
    fn check_scopes(&self, pid: NodeId, g: &Graph, p: &ast::Proc) -> Result<(), Error> {
        let ancestors = |mut r: RegionId| {
            let mut set = HashSet::from([r]);
            while let Some(parent) = g.regions[r].parent {
                set.insert(parent);
                r = parent;
            }
            set
        };
        let check = |scope: RegionId, r: &Ref| -> Result<(), Error> {
            let Root::Node(n) = r.root else { return Ok(()) };
            match g.region_of.get(&n) {
                Some(nr) if ancestors(scope).contains(nr) => Ok(()),
                Some(_) => err(r.span, format!("{} runs inside a for each region, so its output is not visible here", self.prog.nodes[n].qual_name())),
                None => err(r.span, format!("{} never runs in this proc", self.prog.nodes[n].qual_name())),
            }
        };
        for (&src, edge) in &g.edges {
            let Some(&r) = g.region_of.get(&src) else { continue };
            let scope = match edge {
                Edge::ForEach { dim, .. } => {
                    check(r, dim)?;
                    g.opens[&src]
                }
                Edge::Collect(_) => g.regions[r].parent.unwrap(),
                _ => r,
            };
            for c in edge.calls() {
                for a in &c.args {
                    let mut refs = vec![];
                    a.value.refs(&mut refs);
                    for r in refs {
                        check(scope, r)?;
                    }
                }
            }
        }
        let want = &self.prog.proc(pid).ret;
        for &n in &g.regions[0].nodes {
            if !g.edges.contains_key(&n) && !want.accepts(self.prog.ret(n), &self.prog.types) {
                return err(p.name.span, format!(
                    "{} ends proc {} but returns {}, expected {want}",
                    self.prog.nodes[n].qual_name(), p.name.name, self.prog.ret(n),
                ));
            }
        }
        Ok(())
    }
}

fn new_region(g: &mut Graph, parent: Option<RegionId>, entry: NodeId, elem: Type) -> RegionId {
    g.regions.push(Region { parent, entry, elem, nodes: vec![], sink: None });
    g.regions.len() - 1
}
