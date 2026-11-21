//! Leaves for vcc's code generators (../sys/gen): table updates and
//! lookups, names, and text.

use crate::vcc::types::*;

fn err<T>(what: &str) -> Result<T, String> {
    Err(format!("gen leaf: {what}"))
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_ascii_uppercase().to_string() + c.as_str())
}

// ---- The type table ----

pub fn with_type(mut t: Table, n: NamedType) -> Result<Table, String> {
    t.types = t.types.concat(&vec![n].into());
    Ok(t)
}

pub fn with_shape(mut t: Table, i: i64, s: Shape) -> Result<Table, String> {
    let mut v: Vec<NamedType> = t.types.into_iter().collect();
    v[i as usize].shape = s;
    t.types = v.into();
    Ok(t)
}

pub fn with_key(mut t: Table, key: Str, i: i64) -> Result<Table, String> {
    t.by_key = t.by_key.merge(&Map::from_iter([(key, i)]));
    Ok(t)
}

pub fn with_alias_ref(mut t: Table, a: Alias) -> Result<Table, String> {
    t.aliases = t.aliases.concat(&vec![a].into());
    Ok(t)
}

pub fn with_boxed(mut t: Table, boxed: Map<Str, bool>) -> Result<Table, String> {
    t.boxed = boxed;
    Ok(t)
}

pub fn reserved(types: List<TypeDef>) -> Result<Map<Str, bool>, String> {
    Ok(types.iter().map(|d| (Str::from(cap(&d.ident)), true)).collect())
}

pub fn unique(t: Table, hint: Str, typedef: Str) -> Result<Str, String> {
    let taken = |n: &str| {
        t.types.iter().any(|x| &*x.name == n) || t.aliases.iter().any(|a| &*a.name == n) || (t.reserved.contains_key(n) && &*typedef != n)
    };
    let mut name = hint.to_string();
    let mut i = 2;
    while taken(&name) {
        name = format!("{hint}{i}");
        i += 1;
    }
    Ok(name.into())
}

fn prim_key(p: &str) -> String {
    cap(p)
}

fn key(ty: &Type, defs: &[TypeDef]) -> String {
    match ty {
        Type::Prim(p) => prim_key(p),
        Type::Array(e) => format!("[]{}", key(e, defs)),
        Type::Map(m) => format!("map[{}]{}", key(&m.key, defs), key(&m.value, defs)),
        Type::Named(n) if defs[n.id as usize].recursive => format!("#{}", n.id),
        Type::Named(n) => key(&defs[n.id as usize].ty, defs),
        Type::Struct(fs) | Type::Enum(fs) => {
            let mut parts: Vec<String> = fs.iter().map(|m| format!("{}:{}", m.name, key(&m.ty, defs))).collect();
            parts.sort();
            let kind = if matches!(ty, Type::Struct(_)) { "struct" } else { "enum" };
            format!("{kind}{{{}}}", parts.join(","))
        }
    }
}

pub fn shape_key(t: Type, defs: List<TypeDef>) -> Result<Str, String> {
    Ok(key(&t, &defs).into())
}

pub fn get(t: Table, ty: Type) -> Result<TyRef, String> {
    fn go(t: &Table, ty: &Type) -> Result<TyRef, String> {
        Ok(match ty {
            Type::Prim(p) => TyRef::Prim(p.clone()),
            Type::Array(e) => TyRef::Array(std::sync::Arc::new(go(t, e)?)),
            Type::Map(m) => TyRef::Map(std::sync::Arc::new(TyRefMap { key: std::sync::Arc::new(go(t, &m.key)?), value: std::sync::Arc::new(go(t, &m.value)?) })),
            Type::Named(n) if !t.defs[n.id as usize].recursive => go(t, &t.defs[n.id as usize].ty)?,
            _ => {
                let k = key(ty, &t.defs);
                TyRef::Named(*t.by_key.get(&Str::from(k.as_str())).ok_or_else(|| format!("unregistered type {k}"))?)
            }
        })
    }
    go(&t, &ty)
}

pub fn is_boxed(t: Table, i: i64, name: Str) -> Result<bool, String> {
    Ok(t.boxed.contains_key(&Str::from(format!("{i}:{name}"))))
}

pub fn find_boxed(t: Table) -> Result<Map<Str, bool>, String> {
    let edges: Vec<Vec<(Str, usize)>> = t
        .types
        .iter()
        .map(|n| {
            let (Shape::Struct(fs) | Shape::Enum(fs)) = &n.shape;
            fs.iter().filter_map(|m| match &m.r {
                TyRef::Named(j) => Some((m.name.clone(), *j as usize)),
                _ => None,
            }).collect()
        })
        .collect();
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
    let mut out = vec![];
    for (i, es) in edges.iter().enumerate() {
        for (n, j) in es {
            if reaches(*j, i) {
                out.push((Str::from(format!("{i}:{n}")), true));
            }
        }
    }
    Ok(out.into_iter().collect())
}

pub fn gname(n: NodeDecl) -> Result<Str, String> {
    Ok(n.qual.iter().map(|s| cap(s)).collect::<String>().into())
}

pub fn func_params(n: NodeDecl) -> Result<List<Member>, String> {
    Ok(match n.kind {
        NodeKind::Func(f) => f.params,
        _ => List::default(),
    })
}

// ---- Text ----

/// Lines rendered as the reference's Writer does: indentation inferred from
/// brackets (a line starting with a closer dedents, one ending with an
/// opener indents; Go `case`/`default:` lines hang one level out). Raw
/// chunks are copied as they are.
pub fn render_lines(lines: List<Line>, indent: Str) -> Result<Str, String> {
    let mut out = String::new();
    let mut depth: usize = 0;
    for l in lines.iter() {
        match l {
            Line::Raw(s) => out.push_str(s),
            Line::L(s) => {
                let s = s.trim();
                if s.starts_with(['}', ')', ']']) {
                    depth = depth.saturating_sub(1);
                }
                let hang = s.starts_with("case ") || s == "default:";
                let d = if hang { depth.saturating_sub(1) } else { depth };
                if !s.is_empty() {
                    out.push_str(&indent.repeat(d));
                }
                out.push_str(s);
                out.push('\n');
                if s.ends_with(['{', '(', '[']) {
                    depth += 1;
                }
            }
        }
    }
    Ok(out.into())
}

const RUST_RUNTIME: &str = r##"
/// Error returned by procs.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// The node that failed, wrapping its implementation's error.
#[derive(Debug)]
pub struct NodeError {
    pub node: &'static str,
    pub source: Error,
}

impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.node, self.source)
    }
}

impl std::error::Error for NodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

fn wrap(node: &'static str, source: Error) -> Error {
    Box::new(NodeError { node, source })
}

fn get<'a, T>(slot: &'a Option<T>, node: &str) -> Result<&'a T, Error> {
    slot.as_ref().ok_or_else(|| format!("{node} has not run").into())
}

/// Moves a value out of its slot, at its last read.
fn take_slot<T>(slot: &mut Option<T>, node: &str) -> Result<T, Error> {
    slot.take().ok_or_else(|| format!("{node} has not run").into())
}

/// Looks up `k`, failing if it's missing.
fn map_get<K: Ord + std::fmt::Debug, V: Clone>(m: &std::collections::BTreeMap<K, V>, k: &K) -> Result<V, Error> {
    m.get(k).cloned().ok_or_else(|| format!("key {k:?} not found").into())
}

/// Indexes `xs`, failing if `i` is out of range.
fn slice_get<E: Clone>(xs: &[E], i: &i64) -> Result<E, Error> {
    usize::try_from(*i)
        .ok()
        .and_then(|i| xs.get(i).cloned())
        .ok_or_else(|| format!("index {i} out of range [0, {})", xs.len()).into())
}

/// Runs `f` over `xs` on scoped threads; results keep input order.
fn fan_out<E: Send, R: Send>(xs: Vec<E>, f: impl Fn(E) -> Result<R, Error> + Sync) -> Result<Vec<R>, Error> {
    std::thread::scope(|s| {
        let f = &f;
        let handles: Vec<_> = xs.into_iter().map(|x| s.spawn(move || f(x))).collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
            .collect()
    })
}

"##;

const RUST_SHARED: &str = r##"
use std::collections::BTreeMap;
use std::sync::Arc;

/// A string. Immutable, so a clone shares it.
pub type Str = Arc<str>;

/// An array. Immutable, so a clone shares it. `concat` reuses this one's
/// buffer when nothing else shares it, and copies otherwise. Derefs to a
/// slice.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct List<T>(Arc<Vec<T>>);

impl<T> Clone for List<T> {
    fn clone(&self) -> Self {
        List(self.0.clone())
    }
}

impl<T> Default for List<T> {
    fn default() -> Self {
        List(Arc::new(Vec::new()))
    }
}

impl<T> std::ops::Deref for List<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T> From<Vec<T>> for List<T> {
    fn from(v: Vec<T>) -> Self {
        List(Arc::new(v))
    }
}

impl<T: Clone> From<&[T]> for List<T> {
    fn from(v: &[T]) -> Self {
        List(Arc::new(v.to_vec()))
    }
}

impl<T> FromIterator<T> for List<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        List(Arc::new(iter.into_iter().collect()))
    }
}

impl<T: Clone> IntoIterator for List<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        Arc::unwrap_or_clone(self.0).into_iter()
    }
}

impl<'a, T> IntoIterator for &'a List<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for List<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Clone> List<T> {
    /// This array's elements, then `other`'s. Appends in place when nothing
    /// else shares this array.
    pub fn concat(mut self, other: &List<T>) -> List<T> {
        Arc::make_mut(&mut self.0).extend_from_slice(other);
        self
    }
}

/// A map. Immutable, so a clone shares it. `merge` and `without` change
/// this one in place when nothing else shares it, and copy otherwise.
/// Derefs to a `BTreeMap`.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Map<K, V>(Arc<BTreeMap<K, V>>);

impl<K, V> Clone for Map<K, V> {
    fn clone(&self) -> Self {
        Map(self.0.clone())
    }
}

impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Map(Arc::new(BTreeMap::new()))
    }
}

impl<K, V> std::ops::Deref for Map<K, V> {
    type Target = BTreeMap<K, V>;
    fn deref(&self) -> &BTreeMap<K, V> {
        &self.0
    }
}

impl<K, V> From<BTreeMap<K, V>> for Map<K, V> {
    fn from(m: BTreeMap<K, V>) -> Self {
        Map(Arc::new(m))
    }
}

impl<K: Ord, V> FromIterator<(K, V)> for Map<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Map(Arc::new(iter.into_iter().collect()))
    }
}

impl<K: Clone, V: Clone> IntoIterator for Map<K, V> {
    type Item = (K, V);
    type IntoIter = std::collections::btree_map::IntoIter<K, V>;
    fn into_iter(self) -> Self::IntoIter {
        Arc::unwrap_or_clone(self.0).into_iter()
    }
}

impl<K: std::fmt::Debug, V: std::fmt::Debug> std::fmt::Debug for Map<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<K: Ord + Clone, V: Clone> Map<K, V> {
    /// This map's entries and `other`'s; `other` wins.
    pub fn merge(mut self, other: &Map<K, V>) -> Map<K, V> {
        Arc::make_mut(&mut self.0).extend(other.iter().map(|(k, v)| (k.clone(), v.clone())));
        self
    }

    /// This map without `keys`.
    pub fn without(mut self, keys: &[K]) -> Map<K, V> {
        if keys.iter().any(|k| self.0.contains_key(k)) {
            let m = Arc::make_mut(&mut self.0);
            for k in keys {
                m.remove(k);
            }
        }
        self
    }
}

"##;

pub fn rust_runtime(_n: i64) -> Result<Str, String> {
    Ok(RUST_RUNTIME.into())
}

pub fn rust_shared(_n: i64) -> Result<Str, String> {
    Ok(RUST_SHARED.into())
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "else", "enum", "extern", "false", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static",
    "struct", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn",
    "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized",
    "virtual", "yield", "try", "gen",
];

fn rust_ident_of(s: &str) -> String {
    if ["self", "Self", "super", "crate"].contains(&s) {
        format!("{s}_")
    } else if RUST_KEYWORDS.contains(&s) {
        format!("r#{s}")
    } else {
        s.to_string()
    }
}

/// A Rust identifier for a vandorf name.
pub fn rust_ident(s: Str) -> Result<Str, String> {
    Ok(rust_ident_of(&s).into())
}

/// A parameter or local, kept clear of the generated locals.
pub fn rust_local(s: Str) -> Result<Str, String> {
    Ok(if ["env", "pc", "this", "input", "results", "elem", "dim"].contains(&&*s) { format!("{s}_").into() } else { rust_ident_of(&s).into() })
}

/// `BarStart` -> `bar_start`
pub fn snake(s: Str) -> Result<Str, String> {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    Ok(out.into())
}

/// A string literal in Rust (and Go) source.
pub fn quote(s: Str) -> Result<Str, String> {
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
    Ok(out.into())
}

/// A type in Rust source; named types prefixed with `qual`.
pub fn rust_render(t: Table, r: TyRef, qual: Str) -> Result<Str, String> {
    fn go(t: &Table, r: &TyRef, q: &str) -> String {
        match r {
            TyRef::Prim(p) => match &**p {
                "void" => "()".into(),
                "int" => "i64".into(),
                "float" => "f64".into(),
                "bool" => "bool".into(),
                _ => format!("{q}Str"),
            },
            TyRef::Array(e) => format!("{q}List<{}>", go(t, e, q)),
            TyRef::Map(m) => format!("{q}Map<{}, {}>", go(t, &m.key, q), go(t, &m.value, q)),
            TyRef::Named(i) => format!("{q}{}", t.types[*i as usize].name),
        }
    }
    Ok(go(&t, &r, &qual).into())
}

pub fn rust_doc(n: NamedType) -> Result<Line, String> {
    Ok(Line::L(if *n.origin == *format!("typedef {}", n.name) { "/// A typedef.".into() } else { format!("/// The type of the {}.", n.origin).into() }))
}

pub fn is_void_ref(r: TyRef) -> Result<bool, String> {
    Ok(matches!(&r, TyRef::Prim(p) if &**p == "void"))
}

pub fn rust_field_type(t: Table, i: i64, m: RefMember) -> Result<Str, String> {
    let s = rust_render(t.clone(), m.r, "".into())?;
    Ok(if t.boxed.contains_key(&Str::from(format!("{i}:{}", m.name))) { format!("std::sync::Arc<{s}>").into() } else { s })
}
