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
//!
//! Typedefs are registered first, so a shape that has a typedef is named
//! after it. A typedef that gets no type of its own (it names a primitive,
//! an array, or a shape another typedef already named) becomes an alias.
//!
//! A recursive typedef is one entry, which its own references point back
//! at. Fields that would make a type contain itself directly (not through an
//! array) are [`TypeTable::is_boxed`]: `Box<T>` in Rust, `*T` in Go.

use super::{exported, gname};
use crate::sema::{NodeKind, Program};
use crate::types::{Prim, Type, TypeDef};
use std::collections::{HashMap, HashSet};

/// A type as backends see it: primitives and arrays inline, structs and
/// enums by index into [`TypeTable::types`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyRef {
    Prim(Prim),
    Array(Box<TyRef>),
    Map(Box<TyRef>, Box<TyRef>),
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
    /// Typedefs without a type of their own: `(name, what it names)`.
    pub aliases: Vec<(String, TyRef)>,
    by_key: HashMap<String, usize>,
    /// `(type, field or variant)` stored behind a pointer.
    boxed: HashSet<(usize, String)>,
    /// Typedef names, kept free for the typedefs themselves.
    reserved: HashSet<String>,
    defs: Vec<TypeDef>,
}

impl TypeTable {
    pub fn build(prog: &Program) -> TypeTable {
        let mut t = TypeTable {
            defs: prog.types.clone(),
            reserved: prog.types.iter().map(|d| exported(&d.name)).collect(),
            ..Default::default()
        };
        for (id, d) in prog.types.iter().enumerate() {
            let name = exported(&d.name);
            match t.register(&Type::Named(id, d.name.clone()), &name, &format!("typedef {}", d.name)) {
                TyRef::Named(i) if t.types[i].name == name => {}
                r => t.aliases.push((name, r)),
            }
        }
        for (id, n) in prog.nodes.iter().enumerate() {
            let g = gname(prog, id);
            let q = n.qual_name();
            if let NodeKind::Func(f) = &n.kind {
                for (p, ty) in &f.params {
                    t.register(ty, &format!("{g}{}", exported(p)), &format!("parameter {p} of {q}"));
                }
            }
            if !matches!(n.kind, NodeKind::Alias(_)) {
                t.register(prog.ret(id), &format!("{g}Out"), &format!("return of {q}"));
            }
        }
        t.find_boxed();
        t
    }

    /// Whether field (or variant payload) `name` of type `i` is stored
    /// behind a pointer.
    pub fn is_boxed(&self, i: usize, name: &str) -> bool {
        self.boxed.contains(&(i, name.to_string()))
    }

    /// The table entry of a struct or enum type.
    pub fn index(&self, ty: &Type) -> Option<usize> {
        match self.get(ty) {
            TyRef::Named(i) => Some(i),
            _ => None,
        }
    }

    /// Boxes every field `i -> j` where `j` leads back to `i` through
    /// fields that hold their type directly.
    fn find_boxed(&mut self) {
        let direct = |t: &NamedType| -> Vec<(String, usize)> {
            let parts = match &t.shape {
                Shape::Struct(fs) | Shape::Enum(fs) => fs,
            };
            parts.iter().filter_map(|(n, r)| match r {
                TyRef::Named(j) => Some((n.clone(), *j)),
                _ => None,
            }).collect()
        };
        let edges: Vec<Vec<(String, usize)>> = self.types.iter().map(direct).collect();
        let reaches = |from: usize, to: usize| {
            let mut seen = vec![false; edges.len()];
            let mut stack = vec![from];
            while let Some(n) = stack.pop() {
                if n == to {
                    return true;
                }
                if !std::mem::replace(&mut seen[n], true) {
                    stack.extend(edges[n].iter().map(|(_, j)| *j));
                }
            }
            false
        };
        for (i, es) in edges.iter().enumerate() {
            for (n, j) in es {
                if reaches(*j, i) {
                    self.boxed.insert((i, n.clone()));
                }
            }
        }
    }

    /// The backend view of `ty`. Every struct and enum in a checked program
    /// is registered by [`TypeTable::build`], since expression types are
    /// always parts of declared types.
    pub fn get(&self, ty: &Type) -> TyRef {
        match ty {
            Type::Prim(p) => TyRef::Prim(*p),
            Type::Array(e) => TyRef::Array(Box::new(self.get(e))),
            Type::Map(k, v) => TyRef::Map(Box::new(self.get(k)), Box::new(self.get(v))),
            Type::Named(id, _) if !self.defs[*id].recursive => self.get(&self.defs[*id].ty),
            Type::Struct(_) | Type::Enum(_) | Type::Named(..) => {
                let key = key(ty, &self.defs);
                TyRef::Named(*self.by_key.get(&key).unwrap_or_else(|| panic!("unregistered type {ty}")))
            }
        }
    }

    /// Registers `ty` and its parts, naming new shapes after `hint`.
    fn register(&mut self, ty: &Type, hint: &str, origin: &str) -> TyRef {
        match ty {
            Type::Prim(p) => TyRef::Prim(*p),
            Type::Array(e) => TyRef::Array(Box::new(self.register(e, &format!("{hint}Item"), origin))),
            Type::Map(k, v) => TyRef::Map(
                Box::new(self.register(k, &format!("{hint}Key"), origin)),
                Box::new(self.register(v, &format!("{hint}Value"), origin)),
            ),
            Type::Named(id, n) => {
                let d = &self.defs[*id];
                let (def, recursive, name) = (d.ty.clone(), d.recursive, exported(n));
                if !recursive {
                    return match &def {
                        Type::Struct(_) | Type::Enum(_) => self.shape(&def, &name, &format!("typedef {n}"), Some(&name), None),
                        _ => self.register(&def, &name, &format!("typedef {n}")),
                    };
                }
                match self.by_key.get(&format!("#{id}")) {
                    Some(&i) => TyRef::Named(i),
                    None => self.shape(&def, &name, &format!("typedef {n}"), Some(&name), Some(*id)),
                }
            }
            Type::Struct(_) | Type::Enum(_) => self.shape(ty, hint, origin, None, None),
        }
    }

    /// Registers a struct or enum. `typedef` is the name of the typedef it
    /// defines, if any, which may use that reserved name; `recursive` is
    /// that typedef's id if its references (`#id`) should point here.
    fn shape(&mut self, ty: &Type, hint: &str, origin: &str, typedef: Option<&str>, recursive: Option<usize>) -> TyRef {
        let (Type::Struct(fs) | Type::Enum(fs)) = ty else { unreachable!() };
        let key = key(ty, &self.defs);
        let existing = self.by_key.get(&key).copied();
        let i = existing.unwrap_or(self.types.len());
        if let Some(id) = recursive {
            self.by_key.insert(format!("#{id}"), i);
        }
        if existing.is_some() {
            return TyRef::Named(i);
        }
        let name = self.unique(hint, typedef);
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

    /// A fresh name based on `hint`. Typedef names are reserved for the
    /// typedef itself.
    fn unique(&self, hint: &str, typedef: Option<&str>) -> String {
        let taken = |n: &str| {
            self.types.iter().any(|t| t.name == n)
                || self.aliases.iter().any(|(a, _)| a == n)
                || (self.reserved.contains(n) && typedef != Some(n))
        };
        let mut name = hint.to_string();
        let mut i = 2;
        while taken(&name) {
            name = format!("{hint}{i}");
            i += 1;
        }
        name
    }
}

/// Canonical structural key: fields and variants sorted by name. Non-recursive
/// typedefs are expanded; a reference to a recursive one is `#id`, except at
/// the top, so its definition and an unfolding written out by hand agree.
fn key(ty: &Type, defs: &[TypeDef]) -> String {
    match ty {
        Type::Prim(p) => format!("{p:?}"),
        Type::Array(e) => format!("[]{}", key(e, defs)),
        Type::Map(k, v) => format!("map[{}]{}", key(k, defs), key(v, defs)),
        Type::Named(id, _) if defs[*id].recursive => format!("#{id}"),
        Type::Named(id, _) => key(&defs[*id].ty, defs),
        Type::Struct(fs) | Type::Enum(fs) => {
            let mut parts: Vec<String> = fs.iter().map(|(n, t)| format!("{n}:{}", key(t, defs))).collect();
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

    #[test]
    fn typedefs_name_shapes_and_box_cycles() {
        let prog = crate::check(
            r#"
            type Expr = enum { Lit(int), Neg(Expr), Add({ l: Expr, r: Expr }), Call({ f: string, args: []Expr }) };
            type Pair = { a: int, b: int };
            type Same = { b: int, a: int };
            type Ids = []int;
            proc P(A): int {
                func A(e: Expr, p: { a: int, b: int }, i: Ids): int;
            }
            "#,
        )
        .unwrap();
        let t = TypeTable::build(&prog);
        let names: Vec<&str> = t.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Expr", "ExprAdd", "ExprCall", "Pair"]);
        let aliases: Vec<&str> = t.aliases.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(aliases, vec!["Same", "Ids"]);
        // Neg holds an Expr directly, and Add's fields do via ExprAdd; args
        // is an array, which needs no box.
        assert!(t.is_boxed(0, "Neg") && t.is_boxed(0, "Add") && t.is_boxed(1, "l") && t.is_boxed(1, "r"));
        assert!(!t.is_boxed(0, "Lit") && !t.is_boxed(2, "args") && !t.is_boxed(0, "Call"));
        // The anonymous parameter type is Pair.
        let a = prog.proc(prog.top_procs[0]).entry;
        let p = &prog.params(a)[1].1;
        assert_eq!(t.index(p), Some(3));
    }
}
