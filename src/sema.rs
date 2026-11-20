//! Name resolution, type checking and for-each region analysis.
//!
//! Turns the [`ast`] into a [`Program`]: every func and proc declaration gets a
//! [`NodeId`], and every proc gets a [`Graph`] whose edges point at node ids
//! and whose argument expressions are typed. Both the document generator and
//! the code generator work from this.

use crate::ast::{self, Span};
use crate::types::{Prim, Type};
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
}

#[derive(Debug)]
pub struct FuncDecl {
    pub params: Vec<(String, Type)>,
    pub ret: Type,
    pub imp: Option<ast::ImplRef>,
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

#[derive(Debug, Clone)]
pub enum Expr {
    Ref(Ref),
    /// `Tag(payload)`, constructing a variant of the parameter's enum type.
    Variant { tag: String, payload: Ref },
}

#[derive(Debug, Clone)]
pub struct Ref {
    pub root: Root,
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
            NodeKind::Func(_) => panic!("{} is not a proc", self.nodes[id].qual_name()),
        }
    }

    /// Parameters of a node. A proc takes its entry's parameters.
    pub fn params(&self, id: NodeId) -> &[(String, Type)] {
        match &self.nodes[id].kind {
            NodeKind::Func(f) => &f.params,
            NodeKind::Proc(p) => self.params(p.entry),
        }
    }

    pub fn ret(&self, id: NodeId) -> &Type {
        match &self.nodes[id].kind {
            NodeKind::Func(f) => &f.ret,
            NodeKind::Proc(p) => &p.ret,
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
        prog: Program { nodes: vec![], globals: BTreeMap::new(), top_procs: vec![] },
        procs: vec![],
        errors: vec![],
    };
    cx.declare_file(file);
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
    procs: Vec<(NodeId, &'a ast::Proc)>,
    errors: Vec<Error>,
}

impl<'a> Cx<'a> {
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
        let decl = FuncDecl { params: params?, ret: ret?, imp: f.imp.clone() };
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
        let mut fields = vec![];
        for f in rest {
            ty = match ty.field(&f.name) {
                Some(t) => t.clone(),
                None => return err(f.span, format!("{ty} has no field {}", f.name)),
            };
            fields.push(f.name.clone());
        }
        Ok(Ref { root, fields, ty, span: path.span })
    }

    fn call(&self, pid: NodeId, t: &ast::Target, this: &Type) -> Result<Call, Error> {
        let target = self.resolve_node(pid, &t.node)?;
        let tname = self.prog.nodes[target].qual_name();
        let params = self.prog.params(target);
        let mut given: HashMap<&str, (&ast::Arg, Expr)> = HashMap::new();
        for a in &t.args {
            let Some((_, pty)) = params.iter().find(|(n, _)| *n == a.name.name) else {
                return err(a.name.span, format!("{tname} has no parameter {}", a.name.name));
            };
            let value = match &a.value {
                ast::ArgValue::Ref(p) => {
                    let r = self.reference(pid, p, this)?;
                    if !pty.accepts(&r.ty) {
                        return err(p.span, format!("{} has type {}, but {tname}.{} expects {pty}", p, r.ty, a.name.name));
                    }
                    Expr::Ref(r)
                }
                ast::ArgValue::Variant { tag, payload } => {
                    let Some(vty) = pty.variant(&tag.name) else {
                        return err(tag.span, format!("{tname}.{} has type {pty}, which has no variant {}", a.name.name, tag.name));
                    };
                    let r = self.reference(pid, payload, this)?;
                    if !vty.accepts(&r.ty) {
                        return err(payload.span, format!("variant {} carries {vty}, found {}", tag.name, r.ty));
                    }
                    Expr::Variant { tag: tag.name.clone(), payload: r }
                }
            };
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
                    let Type::Array(elem) = &dim.ty else {
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
                    let Type::Array(elem) = &dim.ty else { unreachable!() };
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
                    match &a.value {
                        Expr::Ref(r) | Expr::Variant { payload: r, .. } => check(scope, r)?,
                    }
                }
            }
        }
        let want = &self.prog.proc(pid).ret;
        for &n in &g.regions[0].nodes {
            if !g.edges.contains_key(&n) && !want.accepts(self.prog.ret(n)) {
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
