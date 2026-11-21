//! Surface syntax produced by the parser.
//!
//! This is a direct tree of what was written, with spans for diagnostics. It
//! is not name-resolved or typed; see [`crate::sema`] for that.

use chumsky::span::SimpleSpan;

/// Index of a source file in the files being compiled together.
pub type FileId = usize;

/// A byte range in one source file.
pub type Span = SimpleSpan<usize, FileId>;

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
    /// Type parameters: `type R<T> = ..` is generic, and each use with
    /// arguments (`R<int>`) is its own typedef.
    pub params: Vec<Ident>,
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
    /// The file as written, relative to the `.sys` file naming it.
    pub written: String,
    /// The file as the backend sees it: relative to its output directory
    /// once [`crate::sema::Program::rebase_impls`] has run, `written` until then.
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
    /// `R<A, B>`: a generic typedef with type arguments.
    Inst(Ident, Vec<TypeExpr>),
    Array(Box<TypeExpr>),
    /// `map[K]V`
    Map(Box<TypeExpr>, Box<TypeExpr>),
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
    /// `A switch { Tag(x) -> B(..), else -> D(..) }` (patterns), or
    /// `A switch { (cond) -> C(..), else -> D(..) }` (conditions).
    Match(Vec<Arm>),
    /// `A collect each (dim) -> B(..)`
    Collect { dim: Ident, target: Target },
    /// `A for each (path) -> B(..)`
    ForEach { dim: Value, target: Target },
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub head: Head,
    /// The arm's head as written, for diagram labels.
    pub label: String,
    pub span: Span,
    pub body: Body,
}

#[derive(Debug, Clone)]
pub enum Head {
    /// Matches the structure of the source's output.
    Pat(Pat),
    /// `(expr)`: a bool condition.
    Cond(Value),
    Else,
}

#[derive(Debug, Clone)]
pub enum Body {
    Call(Target),
    /// `switch { (cond) -> .., else -> .. }`, with the arm's bindings in scope.
    Switch(Vec<Arm>, Span),
}

/// A structural pattern. It has the shape of the literal that builds the
/// value: `Add({ l = Lit(x), r = _ })`.
#[derive(Debug, Clone)]
pub struct Pat {
    pub kind: PatKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum PatKind {
    /// `_`
    Wild,
    /// `x`: matches anything, naming it.
    Bind(Ident),
    /// `Tag(pat)`, or `Tag()` for a variant without payload.
    Variant { tag: Ident, payload: Option<Box<Pat>> },
    /// `{ a = pat, b = pat }`: every field, `_` for those that don't matter.
    Struct(Vec<(Ident, Pat)>),
    /// `pat | pat`
    Or(Vec<Pat>),
}

#[derive(Debug, Clone)]
pub struct Target {
    pub node: Path,
    pub args: Vec<Arg>,
    /// `as Name`: call a separate instance of `node`, named `Name` in this
    /// proc, with its own output and outgoing edge.
    pub alias: Option<Ident>,
    /// `id(value)`: end the proc with `value` (then `node` is `id`, and
    /// `args` is empty).
    pub id: Option<Value>,
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

/// An expression: an argument, a condition or a guard.
#[derive(Debug, Clone)]
pub enum ValueKind {
    /// `this.x`, `Start.l`
    Ref(Path),
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    /// `Name(..)`: an enum variant (`S(value)`, or `S()` without payload),
    /// or a call of a func (`F(a = value)`). Which one depends on the
    /// expected type, so sema decides.
    Apply { name: Path, args: Vec<ApplyArg> },
    /// `{ a = value, b = value }`
    Struct(Vec<Arg>),
    /// `{ ..base, field = value }`: a struct like base, with those fields.
    Update { base: Box<Value>, args: Vec<Arg> },
    /// `[value, value]`
    Array(Vec<Value>),
    /// `[key: value, key: value]`, or `[:]`
    Map(Vec<(Value, Value)>),
    Unary(UnOp, Box<Value>),
    Binary(BinOp, Box<Value>, Box<Value>),
    /// `value[key]`: a map lookup or array index, failing the proc if the
    /// key isn't there.
    Index(Box<Value>, Box<Value>),
    /// `value.name` after something other than a path (`m[k].x`).
    Field(Box<Value>, Ident),
    /// `key in map`
    In(Box<Value>, Box<Value>),
}

#[derive(Debug, Clone)]
pub struct ApplyArg {
    pub name: Option<Ident>,
    pub value: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl BinOp {
    /// The operator as written, which is also its Go and Rust spelling.
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Or => "||",
            BinOp::And => "&&",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
        }
    }
}
