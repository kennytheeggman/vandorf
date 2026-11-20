//! Structural types.
//!
//! Struct fields and enum variants keep source order; equality and
//! [`Type::accepts`] ignore it.

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

#[derive(Debug, Clone)]
pub enum Type {
    Prim(Prim),
    Array(Box<Type>),
    Struct(Vec<(String, Type)>),
    Enum(Vec<(String, Type)>),
}

pub const VOID: Type = Type::Prim(Prim::Void);

fn lookup<'a>(fields: &'a [(String, Type)], name: &str) -> Option<&'a Type> {
    fields.iter().find(|(n, _)| n == name).map(|(_, t)| t)
}

impl Type {
    /// Can a value of type `other` be passed where `self` is expected?
    ///
    /// Structs must have the same field set; an enum accepts any enum whose
    /// variants are a subset of its own.
    pub fn accepts(&self, other: &Type) -> bool {
        match (self, other) {
            (Type::Prim(a), Type::Prim(b)) => a == b,
            (Type::Array(a), Type::Array(b)) => a.accepts(b),
            (Type::Struct(a), Type::Struct(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(n, t)| lookup(b, n).is_some_and(|u| t.accepts(u)))
            }
            (Type::Enum(a), Type::Enum(b)) => {
                b.iter().all(|(n, u)| lookup(a, n).is_some_and(|t| t.accepts(u)))
            }
            _ => false,
        }
    }

    pub fn field(&self, name: &str) -> Option<&Type> {
        match self {
            Type::Struct(fs) => lookup(fs, name),
            _ => None,
        }
    }

    pub fn variant(&self, name: &str) -> Option<&Type> {
        match self {
            Type::Enum(vs) => lookup(vs, name),
            _ => None,
        }
    }

    pub fn is_void(&self) -> bool {
        matches!(self, Type::Prim(Prim::Void))
    }
}

impl PartialEq for Type {
    fn eq(&self, other: &Type) -> bool {
        match (self, other) {
            (Type::Enum(a), Type::Enum(b)) => a.len() == b.len() && self.accepts(other),
            _ => self.accepts(other),
        }
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
    const INT: Type = Type::Prim(Prim::Int);

    #[test]
    fn struct_order_is_irrelevant() {
        let a = s(&[("x", INT), ("y", VOID)]);
        let b = s(&[("y", VOID), ("x", INT)]);
        assert!(a.accepts(&b) && a == b);
        assert!(!a.accepts(&s(&[("x", INT)])));
    }

    #[test]
    fn enum_widens() {
        let wide = e(&[("A", INT), ("B", VOID)]);
        let narrow = e(&[("B", VOID)]);
        assert!(wide.accepts(&narrow));
        assert!(!narrow.accepts(&wide));
        assert!(wide != narrow);
    }
}
