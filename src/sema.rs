//! Name resolution, type checking and for-each region analysis.
//!
//! Turns the [`ast`] into a [`Program`]: every func and proc declaration gets a
//! [`NodeId`], and every proc gets a [`Graph`] whose edges point at node ids
//! and whose argument expressions are typed. Both the document generator and
//! the code generator work from this.

use crate::ast::{self, Span};
pub use crate::ast::{BinOp, UnOp};
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
    /// Arms in source order; the first that matches is taken. Sema checks
    /// that some arm always matches.
    Match(Vec<Arm>),
    /// `dim` is an array of `elem`s.
    ForEach { dim: Expr, elem: Type, call: Call },
    Collect(Call),
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub test: Test,
    /// The arm's head as written.
    pub label: String,
    pub body: Body,
}

#[derive(Debug, Clone)]
pub enum Test {
    Pat(Pat),
    Cond(Expr),
    Else,
}

#[derive(Debug, Clone)]
pub enum Body {
    Call(Call),
    /// A switch of conditions, with the arm's bindings in scope.
    Switch(Vec<Arm>),
}

/// A checked pattern; `ty` is the type of the value it matches.
#[derive(Debug, Clone)]
pub struct Pat {
    pub kind: PatKind,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub enum PatKind {
    Wild,
    Bind(String),
    /// `payload` is `None` for variants that carry void.
    Variant { tag: String, payload: Option<Box<Pat>> },
    /// Every field, in the type's order.
    Struct(Vec<(String, Pat)>),
    Or(Vec<Pat>),
}

impl Arm {
    /// The calls this arm can end in, with the arm heads leading to each.
    pub fn leaves(&self) -> Vec<(Vec<&str>, &Call)> {
        match &self.body {
            Body::Call(c) => vec![(vec![self.label.as_str()], c)],
            Body::Switch(arms) => arms
                .iter()
                .flat_map(|a| a.leaves())
                .map(|(mut path, c)| {
                    path.insert(0, self.label.as_str());
                    (path, c)
                })
                .collect(),
        }
    }

    /// Conditions in this arm and the switches nested in it.
    pub fn conds(&self) -> Vec<&Expr> {
        let mut out = vec![];
        if let Test::Cond(e) = &self.test {
            out.push(e);
        }
        if let Body::Switch(arms) = &self.body {
            out.extend(arms.iter().flat_map(|a| a.conds()));
        }
        out
    }
}

impl Edge {
    pub fn calls(&self) -> Vec<&Call> {
        match self {
            Edge::Call(c) | Edge::Collect(c) | Edge::ForEach { call: c, .. } => vec![c],
            Edge::Match(arms) => arms.iter().flat_map(|a| a.leaves()).map(|(_, c)| c).collect(),
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
    /// `!e` on a bool, `-e` on an int or float.
    Unary(UnOp, Box<Expr>),
    /// `operand` is the type of both operands; the result is bool for
    /// comparisons and logic, `operand` otherwise.
    Binary { op: BinOp, l: Box<Expr>, r: Box<Expr>, operand: Prim },
    /// A func's implementation, called directly rather than as a node.
    Call(Call),
    /// `[k: v]`, at the expected map type.
    Map(Vec<(Expr, Expr)>),
    /// `m[k]` or `xs[i]`, failing if it isn't there; `ty` is `e`'s map or
    /// array type.
    Index { e: Box<Expr>, key: Box<Expr>, ty: Type },
    /// A field of a struct value that isn't a [`Ref`] (`m[k].x`); `ty` is
    /// the struct's type.
    Field { e: Box<Expr>, ty: Type, name: String },
    /// `k in m`
    In { key: Box<Expr>, map: Box<Expr> },
    /// `.len` of a map, array or string, `.keys` or `.values` of a map (in
    /// key order); `ty` is `e`'s type.
    Query { e: Box<Expr>, ty: Type, q: Query },
    /// `m + n`: a new map with both, `n` winning; `ty` is the map type.
    Merge { l: Box<Expr>, r: Box<Expr>, ty: Type },
    /// `m - keys`: a new map without those keys; `ty` is the map type.
    Remove { m: Box<Expr>, keys: Box<Expr>, ty: Type },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query {
    Len,
    Keys,
    Values,
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
            Expr::Unary(_, e) => e.refs(out),
            Expr::Binary { l, r, .. } => {
                l.refs(out);
                r.refs(out);
            }
            Expr::Call(c) => c.args.iter().for_each(|a| a.value.refs(out)),
            Expr::Map(kvs) => kvs.iter().for_each(|(k, v)| {
                k.refs(out);
                v.refs(out);
            }),
            Expr::Field { e, .. } | Expr::Query { e, .. } => e.refs(out),
            Expr::Index { e: a, key: b, .. } | Expr::In { key: a, map: b } | Expr::Merge { l: a, r: b, .. } | Expr::Remove { m: a, keys: b, .. } => {
                a.refs(out);
                b.refs(out);
            }
        }
    }

    /// Whether evaluating this can fail: it calls an implementation, or
    /// looks up a key that may be missing.
    pub fn can_fail(&self) -> bool {
        match self {
            Expr::Call(_) | Expr::Index { .. } => true,
            Expr::Ref(_) | Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) => false,
            Expr::Variant { payload, .. } => payload.as_ref().is_some_and(|p| p.can_fail()),
            Expr::Struct(fs) => fs.iter().any(|(_, e)| e.can_fail()),
            Expr::Array(es) => es.iter().any(|e| e.can_fail()),
            Expr::Map(kvs) => kvs.iter().any(|(k, v)| k.can_fail() || v.can_fail()),
            Expr::Unary(_, e) | Expr::Field { e, .. } | Expr::Query { e, .. } => e.can_fail(),
            Expr::Binary { l: a, r: b, .. } | Expr::In { key: a, map: b } | Expr::Merge { l: a, r: b, .. } | Expr::Remove { m: a, keys: b, .. } => {
                a.can_fail() || b.can_fail()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Ref {
    pub root: Root,
    /// Type of the root (`this`, the node's output, or the binding).
    pub root_ty: Type,
    /// The binding's name, for [`Root::Bind`].
    pub bind: Option<String>,
    pub fields: Vec<String>,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    This,
    /// The output of a node that already ran.
    Node(NodeId),
    /// A variant payload bound by a switch arm (`Lit(n) -> ..`); the name
    /// is [`Ref::bind`].
    Bind,
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
    fn arm_targets<'t>(arms: &'t [ast::Arm], out: &mut Vec<&'t ast::Target>) {
        for a in arms {
            match &a.body {
                ast::Body::Call(t) => out.push(t),
                ast::Body::Switch(inner, _) => arm_targets(inner, out),
            }
        }
    }
    match k {
        ast::EdgeKind::Call(t) | ast::EdgeKind::Collect { target: t, .. } | ast::EdgeKind::ForEach { target: t, .. } => vec![t],
        ast::EdgeKind::Match(arms) => {
            let mut out = vec![];
            arm_targets(arms, &mut out);
            out
        }
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
            K::Map(k, v) => {
                let (kt, vt) = (self.ty(k), self.ty(v));
                let kt = kt?;
                if !matches!(kt.unfold(&self.prog.types), Type::Prim(Prim::Int | Prim::String)) {
                    self.errors.push(Error { msg: format!("map keys are ints or strings, not {kt}"), span: k.span });
                    return None;
                }
                Type::Map(Box::new(kt), Box::new(vt?))
            }
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

    /// A path's value: struct fields as a [`Ref`], then any pseudo-fields
    /// (`.len`, `.keys`, `.values`).
    fn path_value(&self, pid: NodeId, path: &ast::Path, sc: &Scope) -> Result<(Expr, Type), Error> {
        let (r, rest) = self.reference_upto(pid, path, sc)?;
        let mut t = r.ty.clone();
        let mut e = Expr::Ref(r);
        for f in rest {
            (e, t) = self.select(e, &t, f)?;
        }
        Ok((e, t))
    }

    /// `e.name` where `e` has type `t`: a struct field or a pseudo-field.
    fn select(&self, e: Expr, t: &Type, name: &ast::Ident) -> Result<(Expr, Type), Error> {
        let defs = &self.prog.types;
        let query = |q| Expr::Query { e: Box::new(e.clone()), ty: t.clone(), q };
        Ok(match (t.unfold(defs), name.name.as_str()) {
            (Type::Struct(_), f) if t.field(f, defs).is_some() => {
                let ft = t.field(f, defs).unwrap().clone();
                (Expr::Field { e: Box::new(e), ty: t.clone(), name: f.to_string() }, ft)
            }
            (Type::Map(..) | Type::Array(_) | Type::Prim(Prim::String), "len") => (query(Query::Len), Type::Prim(Prim::Int)),
            (Type::Map(k, _), "keys") => (query(Query::Keys), Type::Array(k.clone())),
            (Type::Map(_, v), "values") => (query(Query::Values), Type::Array(v.clone())),
            _ => return err(name.span, format!("{t} has no field {}", name.name)),
        })
    }

    /// The root and struct fields of a path, stopping at the first segment
    /// that isn't a struct field. The rest are returned.
    fn reference_upto<'p>(&self, pid: NodeId, path: &'p ast::Path, sc: &Scope) -> Result<(Ref, &'p [ast::Ident]), Error> {
        let first = &path.segs[0].name;
        let (root, mut ty, rest, bind) = if first == "this" {
            (Root::This, sc.this.clone(), &path.segs[1..], None)
        } else if let Some((_, t)) = sc.binds.iter().find(|(n, _)| n == first) {
            (Root::Bind, t.clone(), &path.segs[1..], Some(first.clone()))
        } else {
            let (n, rest) = self.resolve(pid, path)?;
            (Root::Node(n), self.prog.ret(n).clone(), rest, None)
        };
        let root_ty = ty.clone();
        let mut fields = vec![];
        let mut i = 0;
        while let Some(f) = rest.get(i) {
            if !matches!(ty.unfold(&self.prog.types), Type::Struct(_)) {
                break;
            }
            ty = match ty.field(&f.name, &self.prog.types) {
                Some(t) => t.clone(),
                None => return err(f.span, format!("{ty} has no field {}", f.name)),
            };
            fields.push(f.name.clone());
            i += 1;
        }
        Ok((Ref { root, root_ty, bind, fields, ty, span: path.span }, &rest[i..]))
    }

    /// Checks `v` against the type it's passed as. Literals of structs,
    /// arrays and variants take their type from `want`; anything else
    /// computes its own type, which `want` must accept.
    fn check(&self, pid: NodeId, v: &ast::Value, want: &Type, sc: &Scope) -> Result<Expr, Error> {
        use ast::ValueKind as V;
        let defs = &self.prog.types;
        let shape = want.unfold(defs);
        let mismatch = |found: &str| err(v.span, format!("expected {want}, found {found}"));
        Ok(match &v.kind {
            V::Int(i) if matches!(shape, Type::Prim(Prim::Float)) => Expr::Float(*i as f64),
            V::Int(_) | V::Float(_) | V::Str(_) | V::Bool(_) => {
                let (e, t) = self.synth(pid, v, sc)?;
                if !want.accepts(&t, defs) {
                    let article = if matches!(v.kind, V::Int(_)) { "an" } else { "a" };
                    return mismatch(&format!("{article} {t} literal"));
                }
                e
            }
            V::Array(items) => {
                let Type::Array(elem) = shape else { return mismatch("an array literal") };
                Expr::Array(items.iter().map(|i| self.check(pid, i, elem, sc)).collect::<Result<_, _>>()?)
            }
            V::Map(kvs) => {
                let Type::Map(kt, vt) = shape else { return mismatch("a map literal") };
                let mut out = vec![];
                let mut seen: Vec<String> = vec![];
                for (k, v) in kvs {
                    let k = self.check(pid, k, kt, sc)?;
                    if let Some(lit) = match &k {
                        Expr::Int(i) => Some(i.to_string()),
                        Expr::Str(s) => Some(format!("{s:?}")),
                        _ => None,
                    } {
                        if seen.contains(&lit) {
                            return err(v.span, format!("key {lit} given twice"));
                        }
                        seen.push(lit);
                    }
                    out.push((k, self.check(pid, v, vt, sc)?));
                }
                Expr::Map(out)
            }
            V::Struct(args) => {
                let Type::Struct(fs) = shape else { return mismatch("a struct literal") };
                let mut given: HashMap<&str, Expr> = HashMap::new();
                for a in args {
                    let Some((_, ft)) = fs.iter().find(|(n, _)| *n == a.name.name) else {
                        return err(a.name.span, format!("{want} has no field {}", a.name.name));
                    };
                    let e = self.check(pid, &a.value, ft, sc)?;
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
            // `Tag(..)` where an enum with that variant is expected.
            V::Apply { name, args } if name.segs.len() == 1 && want.variant(&name.segs[0].name, defs).is_some() => {
                let tag = &name.segs[0];
                let vty = want.variant(&tag.name, defs).unwrap();
                if let Some(a) = args.iter().find_map(|a| a.name.as_ref()) {
                    return err(a.span, format!("variant {} takes its payload without a name: {}(value)", tag.name, tag.name));
                }
                let payload = match args.as_slice() {
                    [] if vty.is_void() => None,
                    [] => return err(v.span, format!("variant {} carries {vty}; write {}(value)", tag.name, tag.name)),
                    [p] => {
                        let e = self.check(pid, &p.value, vty, sc)?;
                        // `S(this)` with a void `this` is allowed, and carries nothing.
                        (!vty.is_void()).then(|| Box::new(e))
                    }
                    _ => return err(v.span, format!("variant {} takes one payload", tag.name)),
                };
                Expr::Variant { tag: tag.name.clone(), payload }
            }
            V::Apply { name, .. } if name.segs.len() == 1 && matches!(shape, Type::Enum(_)) && self.resolve(pid, name).is_err() => {
                return err(name.span, format!("{want} has no variant {}", name.segs[0].name));
            }
            _ => {
                let (e, t) = self.synth(pid, v, sc)?;
                if !want.accepts(&t, defs) {
                    let what = match &v.kind {
                        V::Ref(p) => p.to_string(),
                        _ => "this expression".into(),
                    };
                    return err(v.span, format!("{what} has type {t}, but {want} is expected"));
                }
                e
            }
        })
    }

    /// An expression's own type, for those that have one.
    fn synth(&self, pid: NodeId, v: &ast::Value, sc: &Scope) -> Result<(Expr, Type), Error> {
        use ast::ValueKind as V;
        let defs = &self.prog.types;
        let prim = |p| Type::Prim(p);
        Ok(match &v.kind {
            V::Ref(p) => self.path_value(pid, p, sc)?,
            V::Field(e, name) => {
                let (x, t) = self.synth(pid, e, sc)?;
                self.select(x, &t, name)?
            }
            V::Index(m, k) => {
                let (x, t) = self.synth(pid, m, sc)?;
                let (key, vt) = match t.unfold(defs) {
                    Type::Map(kt, vt) => (self.check(pid, k, kt, sc)?, (**vt).clone()),
                    Type::Array(et) => (self.check(pid, k, &prim(Prim::Int), sc)?, (**et).clone()),
                    _ => return err(m.span, format!("only maps and arrays can be indexed, not {t}")),
                };
                (Expr::Index { e: Box::new(x), key: Box::new(key), ty: t }, vt)
            }
            V::In(k, m) => {
                let (x, t) = self.synth(pid, m, sc)?;
                let Type::Map(kt, _) = t.unfold(defs) else {
                    return err(m.span, format!("in needs a map on the right, found {t}"));
                };
                let key = self.check(pid, k, kt, sc)?;
                (Expr::In { key: Box::new(key), map: Box::new(x) }, prim(Prim::Bool))
            }
            V::Map(_) => {
                return err(v.span, "the type of this literal isn't known here; it can only be passed where a type is expected");
            }
            V::Int(i) => (Expr::Int(*i), prim(Prim::Int)),
            V::Float(f) => (Expr::Float(*f), prim(Prim::Float)),
            V::Str(s) => (Expr::Str(s.clone()), prim(Prim::String)),
            V::Bool(b) => (Expr::Bool(*b), prim(Prim::Bool)),
            V::Struct(_) | V::Array(_) => {
                return err(v.span, "the type of this literal isn't known here; it can only be passed where a type is expected");
            }
            V::Apply { name, args } => {
                let target = match self.resolve_node(pid, name) {
                    Ok(n) => n,
                    Err(_) if name.segs.len() == 1 => {
                        return err(name.span, format!("{name} is neither a func nor a variant of the expected type"));
                    }
                    Err(e) => return Err(e),
                };
                if !matches!(self.prog.nodes[target].kind, NodeKind::Func(_)) {
                    return err(name.span, format!("{name} is not a func; only funcs can be called in expressions"));
                }
                if self.prog.ret(target).is_void() {
                    return err(name.span, format!("{name} returns nothing, so it can't be used as a value"));
                }
                let mut named = vec![];
                for a in args {
                    match &a.name {
                        Some(n) => named.push((n, &a.value)),
                        None => return err(a.value.span, format!("arguments of {name} need names: {name}(param = value)")),
                    }
                }
                let call = self.args(pid, target, &named, name.span, sc)?;
                (Expr::Call(call), self.prog.ret(target).clone())
            }
            V::Unary(op, e) => match op {
                UnOp::Not => (Expr::Unary(*op, Box::new(self.check(pid, e, &prim(Prim::Bool), sc)?)), prim(Prim::Bool)),
                UnOp::Neg => {
                    let (x, t) = self.synth(pid, e, sc)?;
                    match t.unfold(defs) {
                        Type::Prim(p @ (Prim::Int | Prim::Float)) => (Expr::Unary(*op, Box::new(x)), prim(*p)),
                        _ => return err(e.span, format!("- needs an int or float, found {t}")),
                    }
                }
            },
            V::Binary(op @ (BinOp::And | BinOp::Or), l, r) => {
                let b = prim(Prim::Bool);
                let (l, r) = (self.check(pid, l, &b, sc)?, self.check(pid, r, &b, sc)?);
                (Expr::Binary { op: *op, l: Box::new(l), r: Box::new(r), operand: Prim::Bool }, b)
            }
            V::Binary(op @ (BinOp::Add | BinOp::Sub), lv, rv) if matches!(self.synth(pid, lv, sc)?.1.unfold(defs), Type::Map(..)) => {
                let (l, lt) = self.synth(pid, lv, sc)?;
                let Type::Map(kt, _) = lt.unfold(defs) else { unreachable!() };
                if *op == BinOp::Add {
                    let r = self.check(pid, rv, &lt, sc)?;
                    (Expr::Merge { l: Box::new(l), r: Box::new(r), ty: lt.clone() }, lt)
                } else {
                    let keys = self.check(pid, rv, &Type::Array(kt.clone()), sc)?;
                    (Expr::Remove { m: Box::new(l), keys: Box::new(keys), ty: lt.clone() }, lt)
                }
            }
            V::Binary(op, lv, rv) => {
                let (mut l, lt) = self.synth(pid, lv, sc)?;
                let (mut r, rt) = self.synth(pid, rv, sc)?;
                let as_prim = |t: &Type| match t.unfold(defs) {
                    Type::Prim(p) => Some(*p),
                    _ => None,
                };
                let (mut lp, mut rp) = (as_prim(&lt), as_prim(&rt));
                // An int literal next to a float is a float.
                if let (Expr::Int(i), Some(Prim::Int), Some(Prim::Float)) = (&l, lp, rp) {
                    (l, lp) = (Expr::Float(*i as f64), Some(Prim::Float));
                }
                if let (Expr::Int(i), Some(Prim::Float), Some(Prim::Int)) = (&r, lp, rp) {
                    (r, rp) = (Expr::Float(*i as f64), Some(Prim::Float));
                }
                use Prim::{Bool, Float, Int, String as Str};
                let ok: &[Prim] = match op {
                    BinOp::Add => &[Int, Float, Str],
                    BinOp::Sub | BinOp::Mul | BinOp::Div => &[Int, Float],
                    BinOp::Rem => &[Int],
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => &[Int, Float, Str],
                    BinOp::Eq | BinOp::Ne => &[Int, Float, Str, Bool],
                    BinOp::And | BinOp::Or => unreachable!(),
                };
                let operand = match (lp, rp) {
                    (Some(a), Some(b)) if a == b && ok.contains(&a) => a,
                    _ => {
                        let allowed: Vec<String> = ok.iter().map(|p| prim(*p).to_string()).collect();
                        return err(v.span, format!(
                            "{} needs two operands of the same type ({}), found {lt} and {rt}",
                            op.symbol(), allowed.join(", "),
                        ));
                    }
                };
                let cmp = matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge);
                let t = if cmp { prim(Prim::Bool) } else { prim(operand) };
                (Expr::Binary { op: *op, l: Box::new(l), r: Box::new(r), operand }, t)
            }
        })
    }

    /// A bool condition or guard.
    fn cond(&self, pid: NodeId, v: &ast::Value, sc: &Scope) -> Result<Expr, Error> {
        if let ast::ValueKind::Str(s) = &v.kind {
            return err(v.span, format!("conditions are expressions now: write ({s}), without quotes"));
        }
        self.check(pid, v, &Type::Prim(Prim::Bool), sc).map_err(|mut e| {
            e.msg = format!("condition: {}", e.msg);
            e
        })
    }

    /// The node a target calls: the alias instance for `-> B(..) as B2`.
    fn target(&self, pid: NodeId, t: &ast::Target) -> Result<NodeId, Error> {
        match &t.alias {
            Some(a) => Ok(self.prog.proc(pid).locals.iter().copied().find(|&l| self.prog.nodes[l].name() == a.name).unwrap()),
            None => self.resolve_node(pid, &t.node),
        }
    }

    fn call(&self, pid: NodeId, t: &ast::Target, sc: &Scope) -> Result<Call, Error> {
        let target = self.target(pid, t)?;
        let named: Vec<(&ast::Ident, &ast::Value)> = t.args.iter().map(|a| (&a.name, &a.value)).collect();
        self.args(pid, target, &named, t.node.span, sc)
    }

    /// Checks named arguments against `target`'s parameters.
    fn args(&self, pid: NodeId, target: NodeId, given: &[(&ast::Ident, &ast::Value)], span: Span, sc: &Scope) -> Result<Call, Error> {
        let tname = self.prog.nodes[target].qual_name();
        let params = self.prog.params(target);
        let mut seen: HashMap<&str, Expr> = HashMap::new();
        for (name, value) in given {
            let Some((_, pty)) = params.iter().find(|(n, _)| *n == name.name) else {
                return err(name.span, format!("{tname} has no parameter {}", name.name));
            };
            let e = self.check(pid, value, pty, sc).map_err(|mut e| {
                e.msg = format!("argument {} of {tname}: {}", name.name, e.msg);
                e
            })?;
            if seen.insert(&name.name, e).is_some() {
                return err(name.span, format!("argument {} given twice", name.name));
            }
        }
        let mut args = vec![];
        for (name, pty) in params {
            match seen.remove(name.as_str()) {
                Some(value) => args.push(CallArg { name: name.clone(), param: pty.clone(), value }),
                None => return err(span, format!("call to {tname} is missing argument {name}")),
            }
        }
        Ok(Call { target, args, span })
    }

    /// A switch: arms in order, the first that matches taken. Its arms are
    /// all patterns or all conditions; either way `else` may come last. A
    /// pattern arm may lead to a nested switch of conditions, which sees
    /// the arm's bindings. Some arm always matches: a switch of conditions
    /// ends in `else`, and a pattern switch has an `else` or covers every
    /// value.
    /// `sc.this` is the value matched: the source's output.
    fn switch(&self, pid: NodeId, src: &ast::Path, arms: &[ast::Arm], span: Span, sc: &Scope, nested: bool) -> Result<Vec<Arm>, Error> {
        let ret = sc.this;
        let defs = &self.prog.types;
        let patterns = arms.iter().find(|a| matches!(a.head, ast::Head::Pat(_)));
        let conds = arms.iter().find(|a| matches!(a.head, ast::Head::Cond(_)));
        if let Some(p) = patterns {
            if nested {
                return err(p.span, "a nested switch has nothing new to match, so its arms are conditions: (expr) -> ..");
            }
            if let Some(c) = conds {
                return err(c.span, format!(
                    "this switch matches patterns (like {}), so its arms can't be conditions; put conditions in a switch under a pattern arm: Pat -> switch {{ (cond) -> .., else -> .. }}",
                    p.label,
                ));
            }
        }
        let mut out: Vec<Arm> = vec![];
        for (i, a) in arms.iter().enumerate() {
            if matches!(a.head, ast::Head::Else) && i + 1 < arms.len() {
                return err(arms[i + 1].span, "this arm comes after else, so it's never taken");
            }
            let mut sc = Scope { this: sc.this, binds: sc.binds.clone() };
            let test = match &a.head {
                ast::Head::Else => Test::Else,
                ast::Head::Cond(c) => Test::Cond(self.cond(pid, c, &sc)?),
                ast::Head::Pat(p) => {
                    let mut binds = vec![];
                    let pat = self.pat(pid, p, ret, &mut binds)?;
                    sc.binds.extend(binds);
                    Test::Pat(pat)
                }
            };
            let body = match &a.body {
                ast::Body::Call(t) => Body::Call(self.call(pid, t, &sc)?),
                ast::Body::Switch(inner, ispan) => Body::Switch(self.switch(pid, src, inner, *ispan, &sc, true)?),
            };
            out.push(Arm { test, label: a.label.clone(), body });
        }
        let has_else = matches!(out.last().map(|a| &a.test), Some(Test::Else));
        if patterns.is_none() {
            if !has_else {
                return err(span, format!("this switch on {src} needs an else arm, taken when no condition holds"));
            }
            return Ok(out);
        }
        let pats: Vec<&Pat> = out.iter().filter_map(|a| match &a.test {
            Test::Pat(p) => Some(p),
            _ => None,
        }).collect();
        if let Some(i) = crate::exhaust::unreachable(&pats, ret, defs) {
            return err(arms[i].span, "this arm is never taken: the arms above already match everything it does");
        }
        match crate::exhaust::missing(&pats, ret, defs) {
            Some(w) if !has_else => err(span, format!("switch on {src} doesn't cover {w}; add an arm for it, or an else")),
            None if has_else => err(arms.last().unwrap().span, "else is never taken: the arms above cover every case"),
            _ => Ok(out),
        }
    }

    /// Checks pattern `p` against the type of the value it matches,
    /// collecting its bindings.
    fn pat(&self, pid: NodeId, p: &ast::Pat, want: &Type, binds: &mut Vec<(String, Type)>) -> Result<Pat, Error> {
        use ast::PatKind as K;
        let defs = &self.prog.types;
        let kind = match &p.kind {
            K::Wild => PatKind::Wild,
            K::Bind(b) => {
                if let Some(v) = want.variant(&b.name, defs) {
                    let fix = if v.is_void() { format!("{}()", b.name) } else { format!("{}(_)", b.name) };
                    return err(b.span, format!("{} is a variant of {want}; write {fix} to match it", b.name));
                }
                self.check_binding(pid, b)?;
                if binds.iter().any(|(n, _)| *n == b.name) {
                    return err(b.span, format!("{} is bound twice in this pattern", b.name));
                }
                binds.push((b.name.clone(), want.clone()));
                PatKind::Bind(b.name.clone())
            }
            K::Variant { tag, payload } => {
                if !matches!(want.unfold(defs), Type::Enum(_)) {
                    return err(tag.span, format!("this matches a {want}, which isn't an enum, so {}(..) can't match it", tag.name));
                }
                let Some(vty) = want.variant(&tag.name, defs) else {
                    return err(tag.span, format!("{want} has no variant {}", tag.name));
                };
                let payload = match payload {
                    None if vty.is_void() => None,
                    None => return err(p.span, format!("{} carries {vty}; write {}(_) to ignore it", tag.name, tag.name)),
                    Some(q) if vty.is_void() => return err(q.span, format!("{} carries nothing; write {}()", tag.name, tag.name)),
                    Some(q) => Some(Box::new(self.pat(pid, q, vty, binds)?)),
                };
                PatKind::Variant { tag: tag.name.clone(), payload }
            }
            K::Struct(fields) => {
                let Type::Struct(fs) = want.unfold(defs) else {
                    return err(p.span, format!("this matches a {want}, which isn't a struct"));
                };
                let mut given: HashMap<&str, Pat> = HashMap::new();
                for (n, q) in fields {
                    let Some((_, ft)) = fs.iter().find(|(f, _)| *f == n.name) else {
                        return err(n.span, format!("{want} has no field {}", n.name));
                    };
                    let q = self.pat(pid, q, ft, binds)?;
                    if given.insert(&n.name, q).is_some() {
                        return err(n.span, format!("field {} given twice", n.name));
                    }
                }
                let mut out = vec![];
                for (n, _) in fs {
                    match given.remove(n.as_str()) {
                        Some(q) => out.push((n.clone(), q)),
                        None => return err(p.span, format!("struct pattern is missing field {n} of {want}; write {n} = _ to ignore it")),
                    }
                }
                PatKind::Struct(out)
            }
            K::Or(alts) => {
                let mut out = vec![];
                for a in alts {
                    let mut inner = vec![];
                    out.push(self.pat(pid, a, want, &mut inner)?);
                    if let Some((n, _)) = inner.first() {
                        return err(a.span, format!("or-patterns can't bind names yet ({n})"));
                    }
                }
                PatKind::Or(out)
            }
        };
        Ok(Pat { kind, ty: want.clone() })
    }

    /// Bindings can't hide nodes or `this`.
    fn check_binding(&self, pid: NodeId, b: &ast::Ident) -> Result<(), Error> {
        let path = ast::Path { segs: vec![b.clone()], span: b.span };
        if b.name == "this" || self.resolve(pid, &path).is_ok() {
            return err(b.span, format!("binding {} hides a node or this; pick another name", b.name));
        }
        Ok(())
    }

    fn build_graph(&mut self, pid: NodeId, p: &ast::Proc) -> Result<(), Error> {
        let mut g = Graph::default();
        for e in &p.edges {
            let src = self.resolve_node(pid, &e.source)?;
            let ret = self.prog.ret(src).clone();
            let edge = match &e.kind {
                ast::EdgeKind::Call(t) => Edge::Call(self.call(pid, t, &Scope::this(&ret))?),
                ast::EdgeKind::Match(arms) => Edge::Match(self.switch(pid, &e.source, arms, e.span, &Scope::this(&ret), false)?),
                ast::EdgeKind::Collect { target, .. } => {
                    Edge::Collect(self.call(pid, target, &Scope::this(&Type::Array(Box::new(ret))))?)
                }
                ast::EdgeKind::ForEach { dim, target } => {
                    let (d, t) = self.synth(pid, dim, &Scope::this(&ret))?;
                    let Type::Array(elem) = t.unfold(&self.prog.types) else {
                        return err(dim.span, format!("for each needs an array, found {t}"));
                    };
                    let call = self.call(pid, target, &Scope::this(elem))?;
                    Edge::ForEach { dim: d, elem: (**elem).clone(), call }
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
                Edge::Match(arms) => work.extend(arms.iter().flat_map(|a| a.leaves()).map(|(_, c)| c.target)),
                Edge::Collect(c) => {
                    if r == 0 {
                        return err(c.span, format!("{} collects outside of any for each", name(n)));
                    }
                    if let Some(s) = g.regions[r].sink.filter(|&s| s != n) {
                        return err(c.span, format!("for each region collects at both {} and {}", name(s), name(n)));
                    }
                    g.regions[r].sink = Some(n);
                }
                Edge::ForEach { elem, call, .. } => {
                    let child = new_region(g, Some(r), call.target, elem.clone());
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
            let Root::Node(n) = r.root else { return Ok(()) }; // this and bindings are always in scope
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
                    let mut refs = vec![];
                    dim.refs(&mut refs);
                    for d in refs {
                        check(r, d)?;
                    }
                    g.opens[&src]
                }
                Edge::Collect(_) => g.regions[r].parent.unwrap(),
                _ => r,
            };
            let mut refs = vec![];
            for c in edge.calls() {
                for a in &c.args {
                    a.value.refs(&mut refs);
                }
            }
            if let Edge::Match(arms) = edge {
                for e in arms.iter().flat_map(|a| a.conds()) {
                    e.refs(&mut refs);
                }
            }
            for r in refs {
                check(scope, r)?;
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

/// What names other than nodes mean in an expression.
struct Scope<'t> {
    this: &'t Type,
    /// Switch-arm bindings.
    binds: Vec<(String, Type)>,
}

impl Scope<'_> {
    fn this(this: &Type) -> Scope<'_> {
        Scope { this, binds: vec![] }
    }
}

fn new_region(g: &mut Graph, parent: Option<RegionId>, entry: NodeId, elem: Type) -> RegionId {
    g.regions.push(Region { parent, entry, elem, nodes: vec![], sink: None });
    g.regions.len() - 1
}
