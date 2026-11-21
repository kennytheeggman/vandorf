//! Surface syntax produced by the parser.
//!
//! This is a direct tree of what was written, with spans for diagnostics. It
//! is not name-resolved or typed; see [`crate::sema`] for that.

use chumsky::span::SimpleSpan;

pub type Span = SimpleSpan<usize>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

/// `a.b.c`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub segs: Vec<Ident>,
    pub span: Span,
}

impl std::fmt::Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let segs: Vec<&str> = self.segs.iter().map(|s| s.name.as_str()).collect();
        write!(f, "{}", segs.join("."))
    }
}

#[derive(Debug, Clone, Default)]
pub struct File {
    pub types: Vec<TypeDecl>,
    pub interfaces: Vec<Interface>,
    pub nodes: Vec<Node>,
}

/// `type Name = ty;`
#[derive(Debug, Clone)]
pub struct TypeDecl {
    pub name: Ident,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone)]
pub struct Interface {
    pub name: Ident,
    pub funcs: Vec<Func>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Func(Func),
    Proc(Proc),
}

#[derive(Debug, Clone)]
pub struct Func {
    pub name: Ident,
    pub params: Vec<Field>,
    pub ret: Option<TypeExpr>,
    /// `= "impl.go:Function", "impls.rs:function"`: one per target language,
    /// selected by file extension.
    pub imps: Vec<ImplRef>,
}

/// A `file:function` reference to the code that implements a leaf func.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplRef {
    pub file: String,
    pub function: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Proc {
    pub name: Ident,
    pub entry: Ident,
    pub ret: TypeExpr,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: Ident,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone)]
pub struct TypeExpr {
    pub kind: TypeExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypeExprKind {
    Named(Ident),
    Array(Box<TypeExpr>),
    Struct(Vec<Field>),
    Enum(Vec<Variant>),
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: Ident,
    pub payload: Option<TypeExpr>,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub source: Path,
    pub kind: EdgeKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum EdgeKind {
    /// `A -> B(..)`
    Call(Target),
    /// `A switch { ("cond") -> B(..), .. }`
    Match(Vec<Arm>),
    /// `A collect each (dim) -> B(..)`
    Collect { dim: Ident, target: Target },
    /// `A for each (path) -> B(..)`
    ForEach { dim: Path, target: Target },
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub cond: String,
    pub cond_span: Span,
    pub target: Target,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub node: Path,
    pub args: Vec<Arg>,
    /// `as Name`: call a separate instance of `node`, named `Name` in this
    /// proc, with its own output and outgoing edge.
    pub alias: Option<Ident>,
}

/// `name = value`, in a call or a struct literal.
#[derive(Debug, Clone)]
pub struct Arg {
    pub name: Ident,
    pub value: Value,
}

#[derive(Debug, Clone)]
pub struct Value {
    pub kind: ValueKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ValueKind {
    /// `this.x`, `Start.l`
    Ref(Path),
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    /// `S(value)`, or `S()` for a variant without payload.
    Variant { tag: Ident, payload: Option<Box<Value>> },
    /// `{ a = value, b = value }`
    Struct(Vec<Arg>),
    /// `[value, value]`
    Array(Vec<Value>),
}
