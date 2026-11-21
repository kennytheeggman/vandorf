//! Type table shared by the backends' typegen.
//!
//! Vandorf types are structural; Go and Rust types are nominal. The table
//! gives every distinct struct or enum shape one name, so backends can emit a
//! named type per shape. Shapes are compared like [`Type`]'s equality (field
//! and variant order ignored), and named after the first place they appear:
//!
//! - `Bar.End`'s parameter `r`       -> `BarEndR`
//! - the return of `ForEach.Start`   -> `ForEachStartOut`
//! - field `b` of that, element of an array, variant payload -> suffixes
//!   `B`, `Item`, `<Variant>`
//!
//! Distinct shapes are distinct names even when one coerces into the other
//! (an enum and a wider enum); backends emit conversions where sema coerced.

use super::{exported, gname};
use crate::sema::{NodeKind, Program};
use crate::types::{Prim, Type};
use std::collections::HashMap;

/// A type as backends see it: primitives and arrays inline, structs and
/// enums by index into [`TypeTable::types`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyRef {
    Prim(Prim),
    Array(Box<TyRef>),
    Named(usize),
}

#[derive(Debug, Clone)]
pub enum Shape {
    Struct(Vec<(String, TyRef)>),
    Enum(Vec<(String, TyRef)>),
}

#[derive(Debug, Clone)]
pub struct NamedType {
    pub name: String,
    pub shape: Shape,
    /// Where it was first seen, for doc comments (`return of Bar.Start`).
    pub origin: String,
}

#[derive(Debug, Default)]
pub struct TypeTable {
    /// In first-use order; a type's fields' types come after it.
    pub types: Vec<NamedType>,
    by_key: HashMap<String, usize>,
}

impl TypeTable {
    pub fn build(prog: &Program) -> TypeTable {
        let mut t = TypeTable::default();
        for (id, n) in prog.nodes.iter().enumerate() {
            let g = gname(prog, id);
            let q = n.qual_name();
            if let NodeKind::Func(f) = &n.kind {
                for (p, ty) in &f.params {
                    t.register(ty, &format!("{g}{}", exported(p)), &format!("parameter {p} of {q}"));
                }
            }
            t.register(prog.ret(id), &format!("{g}Out"), &format!("return of {q}"));
        }
        t
    }

    /// The backend view of `ty`. Every struct and enum in a checked program
    /// is registered by [`TypeTable::build`], since expression types are
    /// always parts of declared types.
    pub fn get(&self, ty: &Type) -> TyRef {
        match ty {
            Type::Prim(p) => TyRef::Prim(*p),
            Type::Array(e) => TyRef::Array(Box::new(self.get(e))),
            Type::Struct(_) | Type::Enum(_) => {
                let key = key(ty);
                TyRef::Named(*self.by_key.get(&key).unwrap_or_else(|| panic!("unregistered type {ty}")))
            }
        }
    }

    fn register(&mut self, ty: &Type, hint: &str, origin: &str) -> TyRef {
        match ty {
            Type::Prim(p) => TyRef::Prim(*p),
            Type::Array(e) => TyRef::Array(Box::new(self.register(e, &format!("{hint}Item"), origin))),
            Type::Struct(fs) | Type::Enum(fs) => {
                let key = key(ty);
                if let Some(&i) = self.by_key.get(&key) {
                    return TyRef::Named(i);
                }
                let i = self.types.len();
                let name = self.unique(hint);
                self.by_key.insert(key, i);
                // Reserve the slot so the parent is listed before its parts.
                self.types.push(NamedType { name, shape: Shape::Struct(vec![]), origin: origin.into() });
                let parts: Vec<(String, TyRef)> = fs
                    .iter()
                    .map(|(n, t)| (n.clone(), self.register(t, &format!("{hint}{}", exported(n)), origin)))
                    .collect();
                self.types[i].shape = match ty {
                    Type::Struct(_) => Shape::Struct(parts),
                    _ => Shape::Enum(parts),
                };
                TyRef::Named(i)
            }
        }
    }

    fn unique(&self, hint: &str) -> String {
        let taken = |n: &str| self.types.iter().any(|t| t.name == n);
        let mut name = hint.to_string();
        let mut i = 2;
        while taken(&name) {
            name = format!("{hint}{i}");
            i += 1;
        }
        name
    }
}

/// Canonical structural key: fields and variants sorted by name.
fn key(ty: &Type) -> String {
    match ty {
        Type::Prim(p) => format!("{p:?}"),
        Type::Array(e) => format!("[]{}", key(e)),
        Type::Struct(fs) | Type::Enum(fs) => {
            let mut parts: Vec<String> = fs.iter().map(|(n, t)| format!("{n}:{}", key(t))).collect();
            parts.sort();
            let kind = if matches!(ty, Type::Struct(_)) { "struct" } else { "enum" };
            format!("{kind}{{{}}}", parts.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_structurally_and_names_by_first_use() {
        let prog = crate::check(
            r#"
            proc P(A): int {
                func A(): { x: int, e: enum { L, R({ y: int }) } };
                func B(v: { e: enum { R({ y: int }), L }, x: int }): int;
                func C(v: enum { L }): int;
                A -> B(v = this);
            }
            "#,
        )
        .unwrap();
        let t = TypeTable::build(&prog);
        let names: Vec<&str> = t.types.iter().map(|t| t.name.as_str()).collect();
        // B's parameter is A's return with fields reordered: same type.
        // C's narrower enum is a different type.
        assert_eq!(names, vec!["PAOut", "PAOutE", "PAOutER", "PCV"]);
    }
}
