//! Structural types.
//!
//! Struct fields and enum variants keep source order; [`Type::accepts`]
//! ignores it. A `type Name = ...` declaration ([`TypeDef`]) is a name for a
//! structure, not a new type: `Name` and its definition are interchangeable.
//! Typedefs may be recursive, so checks that look inside a type take the
//! program's typedefs and [`Type::unfold`] names as they go.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prim {
    Void,
    Int,
    Float,
    Bool,
    String,
}

impl Prim {
    pub fn from_name(s: &str) -> Option<Prim> {
        Some(match s {
            "void" => Prim::Void,
            "int" => Prim::Int,
            "float" => Prim::Float,
            "bool" => Prim::Bool,
            "string" => Prim::String,
            _ => return None,
        })
    }
}

pub type TypeId = usize;

#[derive(Debug, Clone)]
pub enum Type {
    Prim(Prim),
    Array(Box<Type>),
    /// `map[K]V`; keys are ints or strings.
    Map(Box<Type>, Box<Type>),
    Struct(Vec<(String, Type)>),
    Enum(Vec<(String, Type)>),
    /// A reference to a typedef, by index into the program's typedefs. The
    /// name is kept for display.
    Named(TypeId, String),
}

/// `type Name = ty;`, or one instance of a generic typedef (`R<int>`).
#[derive(Debug, Clone)]
pub struct TypeDef {
    /// The name as shown in messages: `Name`, or `R<int>` for an instance.
    pub name: String,
    /// An identifier for generated code: `Name`, or `RInt` for `R<int>`.
    pub ident: String,
    pub ty: Type,
    /// Whether the definition refers back to itself, directly or through
    /// other typedefs. Recursive typedefs are always structs or enums.
    pub recursive: bool,
}

pub const VOID: Type = Type::Prim(Prim::Void);

fn lookup<'a>(fields: &'a [(String, Type)], name: &str) -> Option<&'a Type> {
    fields.iter().find(|(n, _)| n == name).map(|(_, t)| t)
}

impl Type {
    /// The structure behind any typedef names. Sema rejects typedef cycles
    /// that never reach a structure (`type A = B; type B = A;`), so this
    /// terminates.
    pub fn unfold<'a>(&'a self, defs: &'a [TypeDef]) -> &'a Type {
        let mut t = self;
        while let Type::Named(id, _) = t {
            t = &defs[*id].ty;
        }
        t
    }

    /// Can a value of type `other` be passed where `self` is expected?
    ///
    /// Structs must have the same field set; an enum accepts any enum whose
    /// variants are a subset of its own. Typedefs are unfolded; two recursive
    /// types are compatible unless some finite unfolding tells them apart.
    pub fn accepts(&self, other: &Type, defs: &[TypeDef]) -> bool {
        accepts(self, other, defs, &mut vec![])
    }

    pub fn field<'a>(&'a self, name: &str, defs: &'a [TypeDef]) -> Option<&'a Type> {
        match self.unfold(defs) {
            Type::Struct(fs) => lookup(fs, name),
            _ => None,
        }
    }

    pub fn variant<'a>(&'a self, name: &str, defs: &'a [TypeDef]) -> Option<&'a Type> {
        match self.unfold(defs) {
            Type::Enum(vs) => lookup(vs, name),
            _ => None,
        }
    }

    /// Void, as written: typedefs can't name void.
    pub fn is_void(&self) -> bool {
        matches!(self, Type::Prim(Prim::Void))
    }

    /// Typedefs this type mentions without going through another typedef.
    pub fn named(&self, out: &mut Vec<TypeId>) {
        match self {
            Type::Prim(_) => {}
            Type::Array(t) => t.named(out),
            Type::Map(k, v) => {
                k.named(out);
                v.named(out);
            }
            Type::Struct(fs) | Type::Enum(fs) => fs.iter().for_each(|(_, t)| t.named(out)),
            Type::Named(id, _) => out.push(*id),
        }
    }
}

/// `assumed` holds pairs of typedefs being compared further up: meeting one
/// again means the comparison has come full circle without a mismatch.
/// Every check below is a conjunction, so keeping an assumption after a
/// failed comparison can't turn the overall answer from false to true.
fn accepts(a: &Type, b: &Type, defs: &[TypeDef], assumed: &mut Vec<(TypeId, TypeId)>) -> bool {
    if let (Type::Named(x, _), Type::Named(y, _)) = (a, b) {
        if x == y || assumed.contains(&(*x, *y)) {
            return true;
        }
        assumed.push((*x, *y));
    }
    match (a.unfold(defs), b.unfold(defs)) {
        (Type::Prim(a), Type::Prim(b)) => a == b,
        (Type::Array(a), Type::Array(b)) => accepts(a, b, defs, assumed),
        (Type::Map(ak, av), Type::Map(bk, bv)) => accepts(ak, bk, defs, assumed) && accepts(av, bv, defs, assumed),
        (Type::Struct(a), Type::Struct(b)) => {
            a.len() == b.len()
                && a.iter().all(|(n, t)| lookup(b, n).is_some_and(|u| accepts(t, u, defs, assumed)))
        }
        (Type::Enum(a), Type::Enum(b)) => {
            b.iter().all(|(n, u)| lookup(a, n).is_some_and(|t| accepts(t, u, defs, assumed)))
        }
        _ => false,
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |f: &mut fmt::Formatter<'_>, fs: &[(String, Type)], enum_: bool| {
            for (i, (n, t)) in fs.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                match (enum_, t.is_void()) {
                    (true, true) => write!(f, "{n}")?,
                    (true, false) => write!(f, "{n}({t})")?,
                    (false, _) => write!(f, "{n}: {t}")?,
                }
            }
            Ok(())
        };
        match self {
            Type::Prim(p) => write!(f, "{}", format!("{p:?}").to_lowercase()),
            Type::Array(t) => write!(f, "[]{t}"),
            Type::Map(k, v) => write!(f, "map[{k}]{v}"),
            Type::Struct(fs) => {
                write!(f, "{{ ")?;
                list(f, fs, false)?;
                write!(f, " }}")
            }
            Type::Enum(vs) => {
                write!(f, "enum {{ ")?;
                list(f, vs, true)?;
                write!(f, " }}")
            }
            Type::Named(_, n) => write!(f, "{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(fs: &[(&str, Type)]) -> Type {
        Type::Struct(fs.iter().map(|(n, t)| (n.to_string(), t.clone())).collect())
    }
    fn e(vs: &[(&str, Type)]) -> Type {
        Type::Enum(vs.iter().map(|(n, t)| (n.to_string(), t.clone())).collect())
    }
    fn named(id: TypeId, n: &str) -> Type {
        Type::Named(id, n.into())
    }
    const INT: Type = Type::Prim(Prim::Int);

    #[test]
    fn struct_order_is_irrelevant() {
        let a = s(&[("x", INT), ("y", VOID)]);
        let b = s(&[("y", VOID), ("x", INT)]);
        assert!(a.accepts(&b, &[]) && b.accepts(&a, &[]));
        assert!(!a.accepts(&s(&[("x", INT)]), &[]));
    }

    #[test]
    fn enum_widens() {
        let wide = e(&[("A", INT), ("B", VOID)]);
        let narrow = e(&[("B", VOID)]);
        assert!(wide.accepts(&narrow, &[]));
        assert!(!narrow.accepts(&wide, &[]));
    }

    #[test]
    fn recursive_typedefs_compare_structurally() {
        // type L1 = enum { Nil, Cons({ h: int, t: L1 }) }, L2 the same, and
        // L3 with a string head.
        let list = |id, n: &str, head: Type| TypeDef {
            name: n.into(),
            ident: n.into(),
            ty: e(&[("Nil", VOID), ("Cons", s(&[("h", head), ("t", named(id, n))]))]),
            recursive: true,
        };
        let defs = [list(0, "L1", INT), list(1, "L2", INT), list(2, "L3", Type::Prim(Prim::String))];
        assert!(named(0, "L1").accepts(&named(1, "L2"), &defs));
        assert!(!named(0, "L1").accepts(&named(2, "L3"), &defs));
        // An unfolding written out by hand is the same type.
        let cons = e(&[("Nil", VOID), ("Cons", s(&[("h", INT), ("t", named(1, "L2"))]))]);
        assert!(named(0, "L1").accepts(&cons, &defs) && cons.accepts(&named(0, "L1"), &defs));
        assert_eq!(named(0, "L1").variant("Cons", &defs).unwrap().field("h", &defs).unwrap().to_string(), "int");
    }
}
