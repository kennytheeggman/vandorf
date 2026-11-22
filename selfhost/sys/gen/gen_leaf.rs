//! Leaves for vcc's code generators (the .sys files here): table updates and
//! lookups, names, and text.

use crate::vcc::types::*;

fn err<T>(what: &str) -> Result<T, String> {
    Err(format!("gen leaf: {what}"))
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| {
        f.to_ascii_uppercase().to_string() + c.as_str()
    })
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

pub fn reserved(types: List<TypeDef>) -> Result<Map<Str, bool>, String> {
    Ok(types
        .iter()
        .map(|d| (Str::from(cap(&d.ident)), true))
        .collect())
}

pub fn unique(t: Table, hint: Str, typedef: Str) -> Result<Str, String> {
    let taken = |n: &str| {
        t.types.iter().any(|x| &*x.name == n)
            || t.aliases.iter().any(|a| &*a.name == n)
            || (t.reserved.contains_key(n) && &*typedef != n)
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
            let mut parts: Vec<String> = fs
                .iter()
                .map(|m| format!("{}:{}", m.name, key(&m.ty, defs)))
                .collect();
            parts.sort();
            let kind = if matches!(ty, Type::Struct(_)) {
                "struct"
            } else {
                "enum"
            };
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
            Type::Map(m) => TyRef::Map(std::sync::Arc::new(TyRefMap {
                key: std::sync::Arc::new(go(t, &m.key)?),
                value: std::sync::Arc::new(go(t, &m.value)?),
            })),
            Type::Named(n) if !t.defs[n.id as usize].recursive => go(t, &t.defs[n.id as usize].ty)?,
            _ => {
                let k = key(ty, &t.defs);
                TyRef::Named(
                    *t.by_key
                        .get(&Str::from(k.as_str()))
                        .ok_or_else(|| format!("unregistered type {k}"))?,
                )
            }
        })
    }
    go(&t, &ty)
}

pub fn find_boxed(t: Table) -> Result<Map<Str, bool>, String> {
    let edges: Vec<Vec<(Str, usize)>> = t
        .types
        .iter()
        .map(|n| {
            let (Shape::Struct(fs) | Shape::Enum(fs)) = &n.shape;
            fs.iter()
                .filter_map(|m| match &m.r {
                    TyRef::Named(j) => Some((m.name.clone(), *j as usize)),
                    _ => None,
                })
                .collect()
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

/// A failure as a program prints it: the error underneath the nodes, and
/// (when it passed through more than one) the nodes, outermost first.
pub fn describe_error(e: &Error) -> String {
    let mut nodes = vec![];
    let mut cur: &(dyn std::error::Error + Send + Sync + 'static) = &**e;
    while let Some(n) = cur.downcast_ref::<NodeError>() {
        nodes.push(n.node);
        cur = &*n.source;
    }
    if nodes.len() > 1 { format!("{cur}\n  (in {})", nodes.join(" > ")) } else { cur.to_string() }
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

/// Unoptimized, a proc's frame holds every node's temporaries at once, so
/// grammar-deep recursion overflows the default 2 MiB; debug builds give
/// their threads more.
fn thread_builder() -> std::thread::Builder {
    let b = std::thread::Builder::new();
    #[cfg(debug_assertions)]
    let b = b.stack_size(64 << 20);
    b
}

/// Runs `f` over `xs` on scoped threads; results keep input order.
fn fan_out<E: Send, R: Send>(xs: Vec<E>, f: impl Fn(E) -> Result<R, Error> + Sync) -> Result<Vec<R>, Error> {
    std::thread::scope(|s| {
        let f = &f;
        let handles: Vec<_> = xs
            .into_iter()
            .map(|x| thread_builder().spawn_scoped(s, move || f(x)).expect("cannot spawn a thread"))
            .collect();
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
    Ok(
        if ["env", "pc", "this", "input", "results", "elem", "dim"].contains(&&*s) {
            format!("{s}_").into()
        } else {
            rust_ident_of(&s).into()
        },
    )
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
    Ok(Line::L(if *n.origin == *format!("typedef {}", n.name) {
        "/// A typedef.".into()
    } else {
        format!("/// The type of the {}.", n.origin).into()
    }))
}

pub fn is_void_ref(r: TyRef) -> Result<bool, String> {
    Ok(matches!(&r, TyRef::Prim(p) if &**p == "void"))
}

pub fn rust_field_type(t: Table, i: i64, m: RefMember) -> Result<Str, String> {
    let s = rust_render(t.clone(), m.r, "".into())?;
    Ok(
        if t.boxed.contains_key(&Str::from(format!("{i}:{}", m.name))) {
            format!("std::sync::Arc<{s}>").into()
        } else {
            s
        },
    )
}

// ---- Rust procs: pure helpers ----

fn unfold<'a>(mut t: &'a Type, defs: &'a [TypeDef]) -> &'a Type {
    while let Type::Named(n) = t {
        t = &defs[n.id as usize].ty;
    }
    t
}

fn member<'a>(t: &'a Type, defs: &'a [TypeDef], name: &str) -> Option<&'a Type> {
    match unfold(t, defs) {
        Type::Struct(ms) | Type::Enum(ms) => ms.iter().find(|m| &*m.name == name).map(|m| &m.ty),
        _ => None,
    }
}

/// A field or variant payload's type.
pub fn member_of(defs: List<TypeDef>, t: Type, name: Str) -> Result<Type, String> {
    member(&t, &defs, &name)
        .cloned()
        .ok_or_else(|| format!("member_of: no {name}"))
}

pub fn unfolded(defs: List<TypeDef>, t: Type) -> Result<Type, String> {
    Ok(unfold(&t, &defs).clone())
}

fn rtype(t: &Table, ty: &Type) -> Result<String, String> {
    let r = get(t.clone(), ty.clone())?;
    Ok(rust_render(t.clone(), r, "types::".into())?.to_string())
}

/// A vandorf type as written in procs.rs.
pub fn rust_type(t: Table, ty: Type) -> Result<Str, String> {
    Ok(rtype(&t, &ty)?.into())
}

fn boxed(t: &Table, ty: &Type, part: &str) -> bool {
    matches!(get(t.clone(), ty.clone()), Ok(TyRef::Named(i)) if t.boxed.contains_key(&Str::from(format!("{i}:{part}"))))
}

/// Whether part (a field, or variant's payload) of ty is behind an Arc.
pub fn ty_boxed(t: Table, ty: Type, part: Str) -> Result<bool, String> {
    Ok(boxed(&t, &ty, &part))
}

/// A Ref's field path: `.a.b`, whether the last field is boxed, and
/// whether an earlier one is (a value can't move out from behind an Arc).
pub fn ref_path(t: Table, r: Ref) -> Result<RefPathOut, String> {
    let mut ty = r.root_ty.clone();
    let (mut is_boxed, mut through, mut path) = (false, false, String::new());
    for f in r.fields.iter() {
        through |= is_boxed;
        is_boxed = boxed(&t, &ty, f);
        ty = member(&ty, &t.defs, f)
            .cloned()
            .ok_or("ref_path: no field")?;
        path.push('.');
        path.push_str(&rust_ident_of(f));
    }
    Ok(RefPathOut {
        path: path.into(),
        boxed: is_boxed,
        through,
    })
}

pub fn float_lit(f: f64) -> Result<Str, String> {
    Ok(format!("{f:?}_f64").into())
}

pub fn in_type(nodes: List<NodeDecl>, n: i64) -> Result<Str, String> {
    match &nodes[n as usize].kind {
        NodeKind::Func(_) | NodeKind::Id(_) => {
            Ok(format!("In{}", gname(nodes[n as usize].clone())?).into())
        }
        NodeKind::Proc(p) => in_type(nodes.clone(), p.entry),
        NodeKind::Alias(b) => in_type(nodes.clone(), *b),
    }
}

pub fn base_of(nodes: List<NodeDecl>, n: i64) -> Result<i64, String> {
    Ok(match nodes[n as usize].kind {
        NodeKind::Alias(b) => b,
        _ => n,
    })
}

/// The func's implementation in a `.rs` file, if any.
pub fn rs_impl(n: NodeDecl, ext: Str) -> Result<MaybeImpl, String> {
    let NodeKind::Func(f) = &n.kind else {
        return err("rs_impl: not a func");
    };
    Ok(f.imps
        .iter()
        .find(|i| i.file.rsplit_once('.').is_some_and(|(_, e)| *e == *ext))
        .map_or(MaybeImpl::None, |i| MaybeImpl::Some(i.clone())))
}

pub fn join_path(base: Str, rel: Str) -> Result<MaybeStr, String> {
    let mut segs: Vec<&str> = base
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    for s in rel.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                if segs.pop().is_none() {
                    return Ok(MaybeStr::None);
                }
            }
            s => segs.push(s),
        }
    }
    Ok(MaybeStr::Some(segs.join("/").into()))
}

/// `impls.rs` -> `crate::impls`, `a/mod.rs` -> `crate::a`, `lib.rs` -> `crate`.
pub fn module_path(file: Str) -> Result<MaybeStr, String> {
    let stem = file.strip_suffix(".rs").unwrap_or(&file);
    let mut segs: Vec<&str> = stem.split('/').filter(|s| !s.is_empty()).collect();
    if segs.last() == Some(&"mod") {
        segs.pop();
    }
    if segs == ["lib"] || segs == ["main"] {
        segs.clear();
    }
    let valid = |s: &str| {
        s.chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
    };
    if !segs.iter().all(|s| valid(s)) {
        return Ok(MaybeStr::None);
    }
    let mut path = String::from("crate");
    for s in segs {
        path.push_str("::");
        path.push_str(&rust_ident_of(s));
    }
    Ok(MaybeStr::Some(path.into()))
}

/// All procs, parents before children, in source order.
pub fn all_procs_ordered(nodes: List<NodeDecl>, top: List<i64>) -> Result<List<i64>, String> {
    let mut out = vec![];
    let mut stack: Vec<i64> = top.iter().rev().copied().collect();
    while let Some(p) = stack.pop() {
        out.push(p);
        let NodeKind::Proc(d) = &nodes[p as usize].kind else {
            continue;
        };
        for &l in d.locals.iter().rev() {
            if matches!(nodes[l as usize].kind, NodeKind::Proc(_)) {
                stack.push(l);
            }
        }
    }
    Ok(out.into())
}

fn field_ref(t: &Table, ty: &Type, x: &str, f: &str) -> String {
    if boxed(t, ty, f) {
        format!("(&*{x}.{})", rust_ident_of(f))
    } else {
        format!("(&{x}.{})", rust_ident_of(f))
    }
}

fn pat_test_of(t: &Table, p: &SPat, x: &str) -> Result<Option<String>, String> {
    Ok(match &*p.kind {
        SPatKind::Wild | SPatKind::Bind(_) => None,
        SPatKind::Variant(v) => {
            let path = format!("{}::{}", rtype(t, &p.ty)?, rust_ident_of(&v.tag));
            let MaybePat::Some(q) = &*v.payload else {
                return Ok(Some(format!("matches!({x}, {path})")));
            };
            let inner = if boxed(t, &p.ty, &v.tag) { "&**p" } else { "p" };
            Some(match pat_test_of(t, q, &format!("({inner})"))? {
                None => format!("matches!({x}, {path}(..))"),
                Some(c) => format!("matches!({x}, {path}(p) if {c})"),
            })
        }
        SPatKind::Struct(fs) => {
            let mut tests = vec![];
            for f in fs.iter() {
                if let Some(c) = pat_test_of(t, &f.pat, &field_ref(t, &p.ty, x, &f.name))? {
                    tests.push(c);
                }
            }
            (!tests.is_empty()).then(|| format!("({})", tests.join(" && ")))
        }
        SPatKind::Or(alts) => {
            let mut tests = vec![];
            for a in alts.iter() {
                match pat_test_of(t, a, x)? {
                    Some(c) => tests.push(c),
                    None => return Ok(None),
                }
            }
            Some(format!("({})", tests.join(" || ")))
        }
    })
}

/// A bool expression testing whether x (a `&T`) matches p; None if it
/// always does.
pub fn pat_test(t: Table, p: SPat, x: Str) -> Result<MaybeStr, String> {
    Ok(pat_test_of(&t, &p, &x)?.map_or(MaybeStr::None, |s| MaybeStr::Some(s.into())))
}

fn pat_binds_of(t: &Table, p: &SPat, x: String, out: &mut Vec<PatBind>) -> Result<(), String> {
    match &*p.kind {
        SPatKind::Wild | SPatKind::Or(_) => {}
        SPatKind::Bind(b) => out.push(PatBind {
            name: b.clone(),
            expr: x.into(),
        }),
        SPatKind::Variant(v) => {
            if let MaybePat::Some(q) = &*v.payload {
                let path = format!("{}::{}", rtype(t, &p.ty)?, rust_ident_of(&v.tag));
                let inner = if boxed(t, &p.ty, &v.tag) { "&**p" } else { "p" };
                pat_binds_of(
                    t,
                    q,
                    format!("(match {x} {{ {path}(p) => {inner}, _ => unreachable!() }})"),
                    out,
                )?;
            }
        }
        SPatKind::Struct(fs) => {
            for f in fs.iter() {
                pat_binds_of(t, &f.pat, field_ref(t, &p.ty, &x, &f.name), out)?;
            }
        }
    }
    Ok(())
}

/// Expressions (of type `&T`) for p's bindings, given x matches p.
pub fn pat_binds(t: Table, p: SPat, x: Str) -> Result<List<PatBind>, String> {
    let mut out = vec![];
    pat_binds_of(&t, &p, x.to_string(), &mut out)?;
    Ok(out.into())
}

fn arm_leaves_of(a: &SArm, out: &mut Vec<Call>) {
    match &a.body {
        SBody::Call(c) => out.push(c.clone()),
        SBody::Switch(arms) => arms.iter().for_each(|a| arm_leaves_of(a, out)),
    }
}

fn conds_of(a: &SArm, out: &mut Vec<Expr>) {
    if let Test::Cond(e) = &a.test {
        out.push(e.clone());
    }
    if let SBody::Switch(arms) = &a.body {
        arms.iter().for_each(|a| conds_of(a, out));
    }
}

fn refs_of(e: &Expr, out: &mut Vec<i64>) {
    let parts = super::sema_leaf::expr_parts(e.clone()).unwrap_or_default();
    if let Expr::Ref(r) = e {
        if let Root::Node(n) = &r.root {
            out.push(*n);
        }
    }
    for p in parts.iter() {
        refs_of(p, out);
    }
}

fn edge_reads(g: &Graph, n: i64) -> Vec<i64> {
    let mut exprs: Vec<Expr> = vec![];
    match g.edges.get(&n) {
        Some(SEdge::Match(arms)) => arms.iter().for_each(|a| conds_of(a, &mut exprs)),
        Some(SEdge::ForEach(f)) => exprs.push(f.dim.clone()),
        _ => {}
    }
    if let Some(e) = g.edges.get(&n) {
        let calls = match e {
            SEdge::Call(c) | SEdge::Collect(c) => vec![c.clone()],
            SEdge::ForEach(f) => vec![f.call.clone()],
            SEdge::Match(arms) => {
                let mut out = vec![];
                arms.iter().for_each(|a| arm_leaves_of(a, &mut out));
                out
            }
        };
        for c in calls {
            exprs.extend(c.args.iter().map(|a| a.value.clone()));
        }
    }
    let mut out = vec![];
    for e in &exprs {
        refs_of(e, &mut out);
    }
    out
}

/// Nodes whose output some edge reads.
pub fn read_nodes(g: Graph) -> Result<Map<i64, bool>, String> {
    Ok(g.edges
        .keys()
        .flat_map(|&n| edge_reads(&g, n))
        .map(|n| (n, true))
        .collect())
}

fn succs(g: &Graph, n: i64) -> Vec<i64> {
    match g.edges.get(&n) {
        None | Some(SEdge::Collect(_)) => vec![],
        Some(SEdge::ForEach(f)) => {
            let sink = g.regions[g.opens[&n] as usize].sink;
            let mut out = vec![f.call.target];
            if let Some(SEdge::Collect(c)) = g.edges.get(&sink) {
                out.push(c.target);
            }
            out
        }
        Some(SEdge::Call(c)) => vec![c.target],
        Some(SEdge::Match(arms)) => {
            let mut out = vec![];
            arms.iter().for_each(|a| arm_leaves_of(a, &mut out));
            out.iter().map(|c| c.target).collect()
        }
    }
}

/// Whether m's slot can still be read once control moves to target.
pub fn live_after(g: Graph, target: i64, m: i64) -> Result<bool, String> {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![target];
    while let Some(x) = stack.pop() {
        if x == m || !seen.insert(x) {
            continue;
        }
        if edge_reads(&g, x).contains(&m) {
            return Ok(true);
        }
        stack.extend(succs(&g, x));
    }
    Ok(false)
}

/// The reads to move: each owner's reads that no later read of the same
/// owner overlaps (one path a prefix of the other); a node's only if its
/// slot is taken.
pub fn moved_reads(seen: List<Seen>, taken: Map<i64, bool>) -> Result<Map<i64, bool>, String> {
    let overlaps = |a: &[Str], b: &[Str]| a.iter().zip(b).all(|(x, y)| x == y);
    let mut out = vec![];
    for i in 0..seen.len() {
        let s = &seen[i];
        let movable = match s.node {
            n if n >= 0 => taken.contains_key(&n),
            _ => true,
        };
        if movable
            && !seen[i + 1..]
                .iter()
                .any(|o| o.owner == s.owner && overlaps(&s.path, &o.path))
        {
            out.push((i as i64, true));
        }
    }
    Ok(out.into_iter().collect())
}

// ---- Rust procs: state and text ----

pub fn g_coercion(mut g: RGen, key: Str, name: Str) -> Result<RGen, String> {
    g.coercions = g.coercions.merge(&Map::from_iter([(key, name)]));
    Ok(g)
}

pub fn g_failed(mut g: RGen, msg: Str) -> Result<RGen, String> {
    if matches!(g.failed, MaybeStr::None) {
        g.failed = MaybeStr::Some(msg);
    }
    Ok(g)
}

pub fn g_fatal(mut g: RGen, msg: Str) -> Result<RGen, String> {
    if matches!(g.fatal, MaybeStr::None) {
        g.fatal = MaybeStr::Some(msg);
    }
    Ok(g)
}

pub fn first_error(g: RGen) -> Result<MaybeStr, String> {
    Ok(match (g.fatal, g.failed) {
        (MaybeStr::Some(e), _) | (MaybeStr::None, MaybeStr::Some(e)) => MaybeStr::Some(e),
        _ => MaybeStr::None,
    })
}

pub fn no_moves(_n: i64) -> Result<Moves, String> {
    Ok(Moves {
        active: false,
        recording: false,
        seen: List::default(),
        moved: Map::default(),
        taken: Map::default(),
        next: 0,
    })
}

pub fn recording(_n: i64) -> Result<Moves, String> {
    Ok(Moves {
        active: true,
        recording: true,
        ..no_moves(0)?
    })
}

pub fn moving(moved: Map<i64, bool>, taken: Map<i64, bool>) -> Result<Moves, String> {
    Ok(Moves {
        active: true,
        recording: false,
        moved,
        taken,
        ..no_moves(0)?
    })
}

pub fn move_read(mut m: Moves, owner: Str, node: i64, path: List<Str>) -> Result<MoveOut, String> {
    if !m.active {
        return Ok(MoveOut { m, moving: false });
    }
    let i = m.next;
    m.next += 1;
    if m.recording {
        m.seen = m.seen.concat(&vec![Seen { owner, node, path }].into());
        return Ok(MoveOut { m, moving: false });
    }
    let moving = m.moved.contains_key(&i);
    Ok(MoveOut { m, moving })
}

pub fn taken_slots(seen: List<Seen>, g: Graph, target: i64) -> Result<Map<i64, bool>, String> {
    let mut out = vec![];
    for s in seen.iter().filter(|s| s.node >= 0) {
        if !live_after(g.clone(), target, s.node)? {
            out.push((s.node, true));
        }
    }
    Ok(out.into_iter().collect())
}

pub fn sorted_keys(taken: Map<i64, bool>) -> Result<List<i64>, String> {
    Ok(taken.keys().copied().collect())
}

pub fn snake_gname(n: NodeDecl) -> Result<Str, String> {
    snake(gname(n)?)
}

fn sn(nodes: &List<NodeDecl>, n: i64) -> String {
    snake_gname(nodes[n as usize].clone()).unwrap().to_string()
}

fn gn(nodes: &List<NodeDecl>, n: i64) -> String {
    gname(nodes[n as usize].clone()).unwrap().to_string()
}

fn qn(nodes: &List<NodeDecl>, n: i64) -> String {
    super::sema_leaf::qual_name(nodes[n as usize].clone())
        .unwrap()
        .to_string()
}

fn q(s: &str) -> String {
    quote(s.into()).unwrap().to_string()
}

pub fn slot_lines(g: RGen, taken: List<i64>) -> Result<List<Line>, String> {
    Ok(taken
        .iter()
        .map(|&n| {
            let id = sn(&g.prog.nodes, n);
            Line::L(
                format!(
                    "let slot_{id} = take_slot(&mut env.out_{id}, {})?;",
                    q(&qn(&g.prog.nodes, n))
                )
                .into(),
            )
        })
        .collect())
}

pub fn bind_lines(bs: List<PatBind>) -> Result<List<Line>, String> {
    Ok(bs
        .iter()
        .map(|b| Line::L(format!("let b_{} = {}.clone();", b.name, b.expr).into()))
        .collect())
}

pub fn coercion_name(at: RCoerceAtS) -> Result<Str, String> {
    Ok(format!("coerce_{}", at.g.coercions.len()).into())
}

pub fn new_coercion(at: RCoerceAtS) -> Result<RGen, String> {
    let key = format!(
        "{} -> {}",
        rtype(&at.g.table, &at.from)?,
        rtype(&at.g.table, &at.to)?
    );
    let name = coercion_name(at.clone())?;
    g_coercion(at.g, key.into(), name)
}

pub fn is_some_e(m: MaybeExpr) -> Result<bool, String> {
    Ok(matches!(m, MaybeExpr::Some(_)))
}

pub fn unwrap_me(m: MaybeExpr) -> Result<Expr, String> {
    match m {
        MaybeExpr::Some(e) => Ok((*e).clone()),
        MaybeExpr::None => err("unwrap_me"),
    }
}

pub fn is_some_s(m: MaybeStr) -> Result<bool, String> {
    Ok(matches!(m, MaybeStr::Some(_)))
}

pub fn unwrap_s(m: MaybeStr) -> Result<Str, String> {
    match m {
        MaybeStr::Some(s) => Ok(s),
        MaybeStr::None => err("unwrap_s"),
    }
}

pub fn pat_of_test(t: Test) -> Result<SPat, String> {
    match t {
        Test::Pat(p) => Ok(p),
        _ => err("pat_of_test"),
    }
}

pub fn ref_of(e: Expr) -> Result<Ref, String> {
    match e {
        Expr::Ref(r) => Ok(r),
        _ => err("ref_of"),
    }
}

pub fn call_of(e: Expr) -> Result<Call, String> {
    match e {
        Expr::Call(c) => Ok(c),
        _ => err("call_of"),
    }
}

pub fn index_ty(e: Expr) -> Result<Type, String> {
    match e {
        Expr::Index(x) => Ok(x.ty.clone()),
        _ => err("index_ty"),
    }
}

pub fn index_key_e(e: Expr) -> Result<Expr, String> {
    match e {
        Expr::Index(x) => Ok((*x.key).clone()),
        _ => err("index_key_e"),
    }
}

pub fn field_ty(e: Expr) -> Result<Type, String> {
    match e {
        Expr::Field(x) => Ok(x.ty.clone()),
        _ => err("field_ty"),
    }
}

pub fn field_name_e(e: Expr) -> Result<Str, String> {
    match e {
        Expr::Field(x) => Ok(x.name.clone()),
        _ => err("field_name_e"),
    }
}

pub fn merge_ty(e: Expr) -> Result<Type, String> {
    match e {
        Expr::Merge(x) => Ok(x.ty.clone()),
        Expr::Remove(x) => Ok(x.ty.clone()),
        _ => err("merge_ty"),
    }
}

pub fn merge_r_e(e: Expr) -> Result<Expr, String> {
    match e {
        Expr::Merge(x) => Ok((*x.r).clone()),
        _ => err("merge_r_e"),
    }
}

pub fn remove_keys_e(e: Expr) -> Result<Expr, String> {
    match e {
        Expr::Remove(x) => Ok((*x.keys).clone()),
        _ => err("remove_keys_e"),
    }
}

pub fn query_ty(e: Expr) -> Result<Type, String> {
    match e {
        Expr::Query(x) => Ok(x.ty.clone()),
        _ => err("query_ty"),
    }
}

pub fn query_q(e: Expr) -> Result<Str, String> {
    match e {
        Expr::Query(x) => Ok(x.q.clone()),
        _ => err("query_q"),
    }
}

pub fn variant_tag_e(e: Expr) -> Result<Str, String> {
    match e {
        Expr::Variant(x) => Ok(x.tag.clone()),
        _ => err("variant_tag_e"),
    }
}

pub fn unary_op(e: Expr) -> Result<Str, String> {
    match e {
        Expr::Unary(x) => Ok(x.op.clone()),
        _ => err("unary_op"),
    }
}

pub fn binary_r_e(e: Expr) -> Result<Expr, String> {
    match e {
        Expr::Binary(x) => Ok((*x.r).clone()),
        _ => err("binary_r_e"),
    }
}

pub fn binary_op_e(e: Expr) -> Result<Str, String> {
    match e {
        Expr::Binary(x) => Ok(x.op.clone()),
        _ => err("binary_op_e"),
    }
}

pub fn binary_operand(e: Expr) -> Result<Str, String> {
    match e {
        Expr::Binary(x) => Ok(x.operand.clone()),
        _ => err("binary_operand"),
    }
}

pub fn in_key_e(e: Expr) -> Result<Expr, String> {
    match e {
        Expr::In(x) => Ok((*x.key).clone()),
        _ => err("in_key_e"),
    }
}

fn params_of(nodes: &List<NodeDecl>, n: i64) -> List<Member> {
    super::sema_leaf::node_params(nodes.clone(), n).unwrap()
}

pub fn input_args(nodes: List<NodeDecl>, n: i64) -> Result<Str, String> {
    Ok(params_of(&nodes, n)
        .iter()
        .map(|p| format!("input.{}", rust_ident_of(&p.name)))
        .collect::<Vec<_>>()
        .join(", ")
        .into())
}

fn graph(nodes: &List<NodeDecl>, pid: i64) -> Graph {
    super::doc_leaf::graph_of(nodes[pid as usize].clone()).unwrap()
}

pub fn node_head(g: RGen, pid: i64, n: i64, run: Str) -> Result<List<Line>, String> {
    let nodes = &g.prog.nodes;
    let id = sn(nodes, n);
    let mut out = vec![
        Line::L(format!("Pc{}::{} => {{", gn(nodes, pid), gn(nodes, n)).into()),
        Line::L(
            format!(
                "let input = take_slot(&mut env.in_{id}, {})?;",
                q(&qn(nodes, n))
            )
            .into(),
        ),
        Line::L(run),
    ];
    if read_nodes(graph(nodes, pid))?.contains_key(&n) {
        out.push(Line::L(
            format!("env.out_{id} = Some(this.clone());").into(),
        ));
    }
    Ok(out.into())
}

pub fn collect_call(g: Graph, n: i64) -> Result<Call, String> {
    let sink = g.regions[g.opens[&n] as usize].sink;
    match g.edges.get(&sink) {
        Some(SEdge::Collect(c)) => Ok(c.clone()),
        _ => err("collect_call"),
    }
}

pub fn for_each_call_of(g: Graph, opener: i64) -> Result<Call, String> {
    match g.edges.get(&opener) {
        Some(SEdge::ForEach(f)) => Ok(f.call.clone()),
        _ => err("for_each_call_of"),
    }
}

pub fn region_fn(g: RGen, pid: i64, r: i64) -> Result<Str, String> {
    Ok(format!("run_{}_region{r}", sn(&g.prog.nodes, pid)).into())
}

pub fn opens_by_region(g: Graph) -> Result<List<i64>, String> {
    let mut opens: Vec<(i64, i64)> = g.opens.iter().map(|(&n, &r)| (n, r)).collect();
    opens.sort_by_key(|&(_, r)| r);
    Ok(opens.into_iter().map(|(n, _)| n).collect())
}

fn ret_of(nodes: &List<NodeDecl>, n: i64) -> Type {
    super::sema_leaf::node_ret(nodes.clone(), n).unwrap()
}

pub fn proc_prelude(g: RGen, pid: i64) -> Result<List<Line>, String> {
    let nodes = &g.prog.nodes;
    let gr = graph(nodes, pid);
    let NodeKind::Proc(p) = &nodes[pid as usize].kind else {
        return err("proc_prelude");
    };
    let name = gn(nodes, pid);
    let (env, pc) = (format!("Env{name}"), format!("Pc{name}"));
    let order: Vec<i64> = gr.regions.iter().flat_map(|r| r.nodes.to_vec()).collect();
    let mut out: Vec<String> = vec![
        format!("// ---- proc {} ----", qn(nodes, pid)),
        "".into(),
        "#[derive(Debug, Clone, Default)]".into(),
        format!("struct {env} {{"),
    ];
    for &n in &order {
        let id = sn(nodes, n);
        out.push(format!("in_{id}: Option<{}>,", in_type(nodes.clone(), n)?));
        out.push(format!(
            "out_{id}: Option<{}>,",
            rtype(&g.table, &ret_of(nodes, n))?
        ));
    }
    out.extend([
        "}".into(),
        "".into(),
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]".into(),
        format!("enum {pc} {{"),
    ]);
    for &n in &order {
        out.push(format!("{},", gn(nodes, n)));
    }
    out.extend(["}".into(), "".into()]);
    let params = params_of(nodes, pid);
    let sig: Vec<String> = params
        .iter()
        .map(|m| {
            Ok(format!(
                "{}: {}",
                rust_local(m.name.clone())?,
                rtype(&g.table, &m.ty)?
            ))
        })
        .collect::<Result<_, String>>()?;
    out.push(format!("/// Runs proc {}.", qn(nodes, pid)));
    out.push(format!(
        "pub fn run_{}({}) -> Result<{}, Error> {{",
        snake(name.clone().into())?,
        sig.join(", "),
        rtype(&g.table, &p.ret)?
    ));
    out.push(format!("let mut env = {env}::default();"));
    let fields: Vec<String> = params
        .iter()
        .map(|m| {
            Ok(format!(
                "{}: {}",
                rust_ident_of(&m.name),
                rust_local(m.name.clone())?
            ))
        })
        .collect::<Result<_, String>>()?;
    out.push(format!(
        "env.in_{} = Some({} {{ {} }});",
        sn(nodes, p.entry),
        in_type(nodes.clone(), p.entry)?,
        fields.join(", ")
    ));
    out.push(format!("let mut pc = {pc}::{};", gn(nodes, p.entry)));
    Ok(out.into_iter().map(|s| Line::L(s.into())).collect())
}

pub fn region_head(g: RGen, pid: i64, opener: i64) -> Result<List<Line>, String> {
    let nodes = &g.prog.nodes;
    let gr = graph(nodes, pid);
    let r = gr.opens[&opener];
    let region = &gr.regions[r as usize];
    let sink = rtype(&g.table, &ret_of(nodes, region.sink))?;
    Ok(vec![
        Line::L(
            format!(
                "/// One element of the for each from {}.",
                qn(nodes, opener)
            )
            .into(),
        ),
        Line::L(
            format!(
                "fn {}(mut env: Env{}, elem: {}) -> Result<{sink}, Error> {{",
                region_fn(g.clone(), pid, r)?,
                gn(nodes, pid),
                rtype(&g.table, &region.elem)?
            )
            .into(),
        ),
        Line::L("let this = elem;".into()),
    ]
    .into())
}

pub fn name_clash(prog: Program) -> Result<MaybeStr, String> {
    let mut seen: std::collections::HashMap<String, String> = Default::default();
    for (id, n) in prog.nodes.iter().enumerate() {
        let s = sn(&prog.nodes, id as i64);
        let qual = qn(&prog.nodes, id as i64);
        if let Some(prev) = seen.insert(s.clone(), qual.clone()) {
            let _ = n;
            return Ok(MaybeStr::Some(
                format!("{prev} and {qual} both map to the Rust name {s}").into(),
            ));
        }
    }
    Ok(MaybeStr::None)
}

pub fn trim_dir(d: MaybeStr) -> Result<MaybeStr, String> {
    Ok(match d {
        MaybeStr::Some(s) => MaybeStr::Some(s.trim_matches('/').into()),
        MaybeStr::None => MaybeStr::None,
    })
}

pub fn bad_module_dir(d: MaybeStr) -> Result<MaybeStr, String> {
    Ok(match d {
        MaybeStr::Some(s) => match module_path(s.clone())? {
            MaybeStr::None => {
                MaybeStr::Some(format!("module dir {s} is not a valid module path").into())
            }
            MaybeStr::Some(_) => MaybeStr::None,
        },
        MaybeStr::None => MaybeStr::None,
    })
}

pub fn mod_rs(_n: i64) -> Result<Str, String> {
    Ok("\
// Code generated by vandorf. DO NOT EDIT.
//! Procs generated by vandorf. Implementations take and return the types in
//! [`types`].

pub mod types;
mod procs;

pub use procs::*;
"
    .into())
}

fn signature(g: &RGen, n: i64, name: &str, types_path: &str) -> Result<String, String> {
    let qual = format!("{types_path}::");
    let params: Vec<String> = params_of(&g.prog.nodes, n)
        .iter()
        .map(|m| {
            Ok(format!(
                "{}: {}",
                rust_local(m.name.clone())?,
                rust_render(
                    g.table.clone(),
                    get(g.table.clone(), m.ty.clone())?,
                    qual.as_str().into()
                )?
            ))
        })
        .collect::<Result<_, String>>()?;
    let ret = rust_render(
        g.table.clone(),
        get(g.table.clone(), ret_of(&g.prog.nodes, n))?,
        qual.as_str().into(),
    )?;
    Ok(format!(
        "pub fn {name}({}) -> Result<{ret}, E>",
        params.join(", ")
    ))
}

pub fn procs_file(g: RGen, body: Str) -> Result<Str, String> {
    let nodes = &g.prog.nodes;
    let mut ls: Vec<Line> = vec![
        Line::L("// Code generated by vandorf. DO NOT EDIT.".into()),
        Line::L("#![allow(unused, non_snake_case, non_camel_case_types, unreachable_patterns, unreachable_code, clippy::all)]".into()),
        Line::L("".into()),
        Line::L("use super::types;".into()),
        Line::Raw(RUST_RUNTIME.into()),
        Line::Raw(body),
    ];
    let l = |s: String| Line::L(s.into());
    for (id, n) in nodes.iter().enumerate() {
        let id = id as i64;
        if matches!(n.kind, NodeKind::Func(_) | NodeKind::Id(_)) {
            ls.push(l(format!("/// Input of {}.", qn(nodes, id))));
            ls.push(l("#[derive(Debug, Clone)]".into()));
            ls.push(l(format!("struct {} {{", in_type(nodes.clone(), id)?)));
            for m in params_of(nodes, id).iter() {
                ls.push(l(format!(
                    "{}: {},",
                    rust_ident_of(&m.name),
                    rtype(&g.table, &m.ty)?
                )));
            }
            ls.push(l("}".into()));
            ls.push(l("".into()));
        }
    }
    for &n in g.stubs.keys() {
        let qual = qn(nodes, n);
        ls.push(l(format!(
            "/// Stands in for {qual}, which names no .rs implementation. Write"
        )));
        ls.push(l(format!("/// `{}`", signature(&g, n, "name", "types")?)));
        ls.push(l(
            "/// and add `= \"file.rs:name\"` to its declaration.".into()
        ));
        let params: Vec<String> = params_of(nodes, n)
            .iter()
            .map(|m| Ok(format!("_: {}", rtype(&g.table, &m.ty)?)))
            .collect::<Result<_, String>>()?;
        ls.push(l(format!(
            "fn unimplemented_{}({}) -> Result<{}, Error> {{",
            sn(nodes, n),
            params.join(", "),
            rtype(&g.table, &ret_of(nodes, n))?
        )));
        ls.push(l(format!(
            "Err({}.into())",
            q(&format!("{qual} is not implemented"))
        )));
        ls.push(l("}".into()));
        ls.push(l("".into()));
    }
    for h in g.helpers.iter() {
        ls.push(Line::Raw(h.clone()));
        ls.push(l("".into()));
    }
    render_lines(ls.into(), "    ".into())
}

// ---- Rust mains ----

/// An executable running proc `entry` (src/codegen/rust.rs: main_artifact):
/// its directory relative to the output module, file name and text.
pub fn rust_main(
    prog: Program,
    t: Table,
    name: Str,
    entry: Str,
    path: Str,
    module_dir: MaybeStr,
    krate: Str,
) -> Result<MainsRustMainOut, String> {
    let fail = |e: String| {
        Ok(MainsRustMainOut {
            dir: "".into(),
            file: "".into(),
            text: "".into(),
            err: MaybeStr::Some(e.into()),
        })
    };
    let nodes = &prog.nodes;
    let Some(pid) = prog.top.iter().copied().find(|&p| {
        nodes[p as usize].qual.last() == Some(&entry)
            && matches!(nodes[p as usize].kind, NodeKind::Proc(_))
    }) else {
        return fail(format!(
            "{name} runs proc {entry}, but there is no top-level proc {entry}"
        ));
    };
    let params = params_of(nodes, pid);
    let argv = params.len() == 1
        && matches!(unfold(&params[0].ty, &prog.types), Type::Array(e) if matches!(unfold(e, &prog.types), Type::Prim(p) if &**p == "string"));
    let mut typed = vec![];
    if !argv {
        for m in params.iter() {
            match unfold(&m.ty, &prog.types) {
                Type::Prim(p) if &**p != "void" => typed.push((m.name.to_string(), p.to_string())),
                _ => {
                    let shown =
                        crate::vcc::run_show_type(m.ty.clone()).map_err(|e| e.to_string())?;
                    return fail(format!(
                        "{entry}'s parameter {} has type {shown}; {name} reads its parameters from the command line, so they must be int, float, bool or string (or a single []string, for all the arguments)",
                        m.name
                    ));
                }
            }
        }
    }
    let usage = if argv {
        format!("usage: {name} <{}...>", params[0].name)
    } else {
        format!(
            "usage: {name}{}",
            typed
                .iter()
                .map(|(n, p)| format!(" <{n}: {p}>"))
                .collect::<String>()
        )
    };
    let (MaybeStr::Some(dir), MaybeStr::Some(mpath)) = (
        &module_dir,
        module_path(module_dir.clone().map_or(s_(""), |d| d))?,
    ) else {
        return fail(format!(
            "{path} goes in the crate's src/, so the output's place under it must be known: run inside a crate or pass --module-dir"
        ));
    };
    if krate.is_empty() {
        return fail(format!(
            "{path} calls the procs through the library crate, whose name is unknown: run inside a crate with a src/lib.rs"
        ));
    }
    let lib = mpath.replacen("crate", &krate, 1);
    let run = format!("{lib}::run_{}", sn(nodes, pid));
    let types = format!("{lib}::types");
    let mut w: Vec<String> = vec![
        "// Code generated by vandorf. DO NOT EDIT.".into(),
        format!("//! Runs proc {entry}, taking its parameters as arguments:"),
        "//!".into(),
        format!("//! ```text\n//! {usage}\n//! ```"),
        "".into(),
        "use std::process::ExitCode;".into(),
        "".into(),
    ];
    if !argv {
        w.push(format!("const USAGE: &str = {};", q(&usage)));
        w.push("".into());
    }
    w.push("fn main() -> ExitCode {".into());
    let mut vars = vec![];
    if argv {
        let var = rust_local(format!("arg_{}", params[0].name).into())?.to_string();
        w.push(format!("let {var}: {types}::List<{types}::Str> = std::env::args().skip(1).map(|a| {types}::Str::from(a.as_str())).collect();"));
        vars.push(var);
    } else {
        w.push("let args: Vec<String> = std::env::args().skip(1).collect();".into());
        w.push(format!("if args.len() != {} {{", typed.len()));
        w.push("eprintln!(\"{USAGE}\");".into());
        w.push("return ExitCode::from(2);".into());
        w.push("}".into());
    }
    for (i, (n, p)) in typed.iter().enumerate() {
        let var = rust_local(format!("arg_{n}").into())?.to_string();
        if p == "string" {
            w.push(format!(
                "let {var} = {types}::Str::from(args[{i}].as_str());"
            ));
        } else {
            let rt = match p.as_str() {
                "int" => "i64",
                "float" => "f64",
                _ => "bool",
            };
            w.push(format!("let {var} = match args[{i}].parse::<{rt}>() {{"));
            w.push("Ok(v) => v,".into());
            w.push("Err(e) => {".into());
            w.push(format!("eprintln!(\"{n}: {{e}}\\n{{USAGE}}\");"));
            w.push("return ExitCode::from(2);".into());
            w.push("}".into());
            w.push("};".into());
        }
        vars.push(var);
    }
    w.push(format!("match {run}({}) {{", vars.join(", ")));
    let ret = ret_of(nodes, pid);
    match unfold(&ret, &prog.types) {
        Type::Prim(p) if &**p == "void" => w.push("Ok(()) => ExitCode::SUCCESS,".into()),
        r => {
            w.push("Ok(result) => {".into());
            w.push(if matches!(r, Type::Prim(_)) {
                "println!(\"{result}\");".into()
            } else {
                "println!(\"{result:?}\");".into()
            });
            w.push("ExitCode::SUCCESS".into());
            w.push("}".into());
        }
    }
    w.push("Err(e) => {".into());
    w.push(format!(
        "eprintln!(\"error: {{}}\", {lib}::describe_error(&e));"
    ));
    for l in ["ExitCode::FAILURE", "}", "}", "}"] {
        w.push(l.into());
    }
    let _ = &t;
    let text = render_lines(
        w.into_iter().map(|l| Line::L(l.into())).collect(),
        "    ".into(),
    )?;
    // src/ is `dir` levels above the module.
    let up = vec![".."; dir.split('/').filter(|x| !x.is_empty()).count()];
    let p = path.trim_matches('/');
    let (sub, file) = p.rsplit_once('/').unwrap_or(("", p));
    let out_dir = up
        .into_iter()
        .chain((!sub.is_empty()).then_some(sub))
        .collect::<Vec<_>>()
        .join("/");
    Ok(MainsRustMainOut {
        dir: out_dir.into(),
        file: file.into(),
        text,
        err: MaybeStr::None,
    })
}

fn s_(x: &str) -> Str {
    x.into()
}

trait MapOr {
    fn map_or(self, d: Str, f: impl FnOnce(Str) -> Str) -> Str;
}

impl MapOr for MaybeStr {
    fn map_or(self, d: Str, f: impl FnOnce(Str) -> Str) -> Str {
        match self {
            MaybeStr::Some(x) => f(x),
            MaybeStr::None => d,
        }
    }
}

// ---- HTTP ----

const HTTP_RUNTIME: &str = r##"// The fixed part of a generated http.rs: JSON values and their parser, the
// Json trait for primitives, lists and maps, and a std-only HTTP server.
// Generated code adds Json impls for the program's types and a handler per
// server. Not compiled here: included as text by codegen::rust.
//
// Requests: a route's proc takes its parameters from a JSON object body,
// or from the query string (each value read as JSON if it parses, else as
// a string). Responses: 200 and the result as JSON; 400 for a request that
// doesn't decode; 500 and {"error": ..} when the proc fails.

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number as written.
    Num(String),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn parse(s: &str) -> Result<Value, String> {
        let mut p = Parser { b: s.as_bytes(), i: 0 };
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() {
            return Err(format!("unexpected input at byte {}", p.i));
        }
        Ok(v)
    }

    fn object(&self, what: &str) -> Result<&[(String, Value)], String> {
        match self {
            Value::Obj(fs) => Ok(fs),
            _ => Err(format!("expected an object for {what}")),
        }
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", c as char, self.i))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.ws();
        match self.b.get(self.i) {
            None => Err("unexpected end of input".into()),
            Some(b'n') => self.word("null", Value::Null),
            Some(b't') => self.word("true", Value::Bool(true)),
            Some(b'f') => self.word("false", Value::Bool(false)),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut xs = vec![];
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Value::Arr(xs));
                }
                loop {
                    xs.push(self.value()?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Value::Arr(xs));
                        }
                        _ => return Err(format!("expected ',' or ']' at byte {}", self.i)),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut fs = vec![];
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Value::Obj(fs));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    fs.push((k, self.value()?));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Obj(fs));
                        }
                        _ => return Err(format!("expected ',' or '}}' at byte {}", self.i)),
                    }
                }
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                let start = self.i;
                self.i += 1;
                while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
                    self.i += 1;
                }
                Ok(Value::Num(String::from_utf8_lossy(&self.b[start..self.i]).into_owned()))
            }
            Some(_) => Err(format!("unexpected character at byte {}", self.i)),
        }
    }

    fn word(&mut self, w: &str, v: Value) -> Result<Value, String> {
        if self.b[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(format!("unexpected input at byte {}", self.i))
        }
    }

    fn string(&mut self) -> Result<String, String> {
        if self.b.get(self.i) != Some(&b'"') {
            return Err(format!("expected a string at byte {}", self.i));
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            match self.b.get(self.i) {
                None => return Err("unterminated string".into()),
                Some(b'"') => {
                    self.i += 1;
                    return String::from_utf8(out).map_err(|e| e.to_string());
                }
                Some(b'\\') => {
                    let c = *self.b.get(self.i + 1).ok_or("unterminated string")?;
                    self.i += 2;
                    match c {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let hex = std::str::from_utf8(self.b.get(self.i..self.i + 4).ok_or("bad \\u escape")?).map_err(|e| e.to_string())?;
                            let cp = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                            self.i += 4;
                            let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                            out.extend_from_slice(ch.to_string().as_bytes());
                        }
                        c => out.push(c),
                    }
                }
                Some(&c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A value that crosses the HTTP boundary as JSON.
pub trait Json: Sized {
    fn to_json(&self, out: &mut String);
    fn from_json(v: &Value) -> Result<Self, String>;
}

impl Json for () {
    fn to_json(&self, out: &mut String) {
        out.push_str("null");
    }
    fn from_json(_: &Value) -> Result<Self, String> {
        Ok(())
    }
}

impl Json for i64 {
    fn to_json(&self, out: &mut String) {
        out.push_str(&self.to_string());
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        match v {
            Value::Num(n) => n.parse().map_err(|_| format!("{n} is not an int")),
            _ => Err("expected an int".into()),
        }
    }
}

impl Json for f64 {
    fn to_json(&self, out: &mut String) {
        if self.is_finite() {
            out.push_str(&format!("{self:?}"));
        } else {
            out.push_str("null");
        }
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        match v {
            Value::Num(n) => n.parse().map_err(|_| format!("{n} is not a float")),
            _ => Err("expected a float".into()),
        }
    }
}

impl Json for bool {
    fn to_json(&self, out: &mut String) {
        out.push_str(if *self { "true" } else { "false" });
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        match v {
            Value::Bool(b) => Ok(*b),
            _ => Err("expected a bool".into()),
        }
    }
}

impl Json for types::Str {
    fn to_json(&self, out: &mut String) {
        write_str(self, out);
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        match v {
            Value::Str(s) => Ok(types::Str::from(s.as_str())),
            _ => Err("expected a string".into()),
        }
    }
}

impl<T: Json> Json for std::sync::Arc<T> {
    fn to_json(&self, out: &mut String) {
        (**self).to_json(out);
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        Ok(std::sync::Arc::new(T::from_json(v)?))
    }
}

impl<T: Json + Clone> Json for types::List<T> {
    fn to_json(&self, out: &mut String) {
        out.push('[');
        for (i, x) in self.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            x.to_json(out);
        }
        out.push(']');
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        match v {
            Value::Arr(xs) => xs.iter().map(T::from_json).collect::<Result<Vec<_>, _>>().map(types::List::from),
            _ => Err("expected an array".into()),
        }
    }
}

/// A map key: maps are JSON objects, so keys are strings.
pub trait JsonKey: Sized + Ord {
    fn key(&self) -> String;
    fn from_key(k: &str) -> Result<Self, String>;
}

impl JsonKey for types::Str {
    fn key(&self) -> String {
        self.to_string()
    }
    fn from_key(k: &str) -> Result<Self, String> {
        Ok(k.into())
    }
}

impl JsonKey for i64 {
    fn key(&self) -> String {
        self.to_string()
    }
    fn from_key(k: &str) -> Result<Self, String> {
        k.parse().map_err(|_| format!("map key {k} is not an int"))
    }
}

impl<K: JsonKey + Clone, V: Json + Clone> Json for types::Map<K, V> {
    fn to_json(&self, out: &mut String) {
        out.push('{');
        for (i, (k, v)) in self.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_str(&k.key(), out);
            out.push(':');
            v.to_json(out);
        }
        out.push('}');
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        let fs = v.object("a map")?;
        fs.iter().map(|(k, v)| Ok((K::from_key(k)?, V::from_json(v)?))).collect()
    }
}

fn field<'a>(fs: &'a [(String, Value)], name: &str) -> Result<&'a Value, String> {
    fs.iter().find(|(k, _)| k == name).map(|(_, v)| v).ok_or_else(|| format!("missing field {name}"))
}

/// A route's parameter, decoded; a failure is a 400.
fn arg<T: Json>(params: &Value, name: &str) -> Result<T, (u16, String)> {
    let fs = params.object("the parameters").map_err(|e| (400, e))?;
    let v = fs.iter().find(|(k, _)| k == name).map(|(_, v)| v).ok_or_else(|| (400, format!("missing parameter {name}")))?;
    T::from_json(v).map_err(|e| (400, format!("parameter {name}: {e}")))
}

fn json<T: Json>(x: &T) -> String {
    let mut out = String::new();
    x.to_json(&mut out);
    out
}

// ---- The server ----

/// A request: header names lowercased, the query string undecoded.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A server's routes: Some(the response) for a route it has.
pub type Handler = fn(&Request) -> Option<Response>;

/// A file served as it is: URL path, content type, contents.
pub type Static = (&'static str, &'static str, &'static str);

fn json_response(status: u16, body: String) -> Response {
    Response { status, headers: vec![("Content-Type".into(), "application/json".into())], body: body.into_bytes() }
}

fn error_response(status: u16, msg: &str) -> Response {
    json_response(status, json(&types::Map::from_iter([(types::Str::from("error"), types::Str::from(msg))])))
}

/// The query string's parameters, decoded.
fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

/// A JSON route: its parameters from a JSON object body, or the query
/// string (each value read as JSON if it parses, else as a string).
fn json_route(req: &Request, call: fn(&Value) -> Result<String, (u16, String)>) -> Response {
    let params = if req.body.trim().is_empty() {
        Value::Obj(query_pairs(&req.query).into_iter().map(|(k, v)| (k, Value::parse(&v).unwrap_or(Value::Str(v)))).collect())
    } else {
        match Value::parse(&req.body) {
            Ok(v) => v,
            Err(e) => return error_response(400, &format!("body: {e}")),
        }
    };
    match call(&params) {
        Ok(text) => json_response(200, text),
        Err((status, msg)) => error_response(status, &msg),
    }
}

/// A server's main: `[--addr host:port]` from the command line (else
/// `addr`, the manifest's), then serves.
pub fn run(addr: &str, handler: Handler, statics: &'static [Static]) {
    if let Some(addr) = listen_addr(addr) {
        serve(&addr, handler, statics);
    }
}

/// A server's main with a setup: the address as for run, then the setup
/// (if it fails, the server exits without listening), then serves.
pub fn run_setup(addr: &str, setup: fn() -> Result<(), String>, handler: Handler, statics: &'static [Static]) {
    let Some(addr) = listen_addr(addr) else { return };
    if let Err(e) = setup() {
        eprintln!("error: setup: {e}");
        std::process::exit(1);
    }
    serve(&addr, handler, statics);
}

/// `--addr host:port` from the command line, else `addr`; None after
/// printing the usage for --help.
fn listen_addr(addr: &str) -> Option<String> {
    let name = std::env::args().next().map(|a| a.rsplit('/').next().unwrap_or("").to_string()).unwrap_or_default();
    let usage = format!("usage: {name} [--addr host:port]   (default {addr})");
    let mut addr = addr.to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match (a.as_str(), args.next()) {
            ("--addr", Some(v)) => addr = v,
            ("-h" | "--help", _) => {
                eprintln!("{usage}");
                return None;
            }
            _ => {
                eprintln!("error: unexpected {a}\n{usage}");
                std::process::exit(2);
            }
        }
    }
    Some(addr)
}

static ADDR: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The address being served on, once serving ("" before).
pub fn addr() -> &'static str {
    ADDR.get().map_or("", String::as_str)
}

/// Serves until the process is stopped, a thread per connection.
pub fn serve(addr: &str, handler: Handler, statics: &'static [Static]) {
    let listener = match std::net::TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: cannot listen on {addr}: {e}");
            std::process::exit(1);
        }
    };
    let _ = ADDR.set(addr.to_string());
    eprintln!("listening on http://{addr}");
    for stream in listener.incoming().flatten() {
        let _ = thread_builder().spawn(move || {
            let _ = connection(stream, handler, statics);
        });
    }
}

/// Unoptimized, a proc's frame holds every node's temporaries at once, so
/// grammar-deep recursion overflows the default 2 MiB; debug builds give
/// their threads more.
fn thread_builder() -> std::thread::Builder {
    let b = std::thread::Builder::new();
    #[cfg(debug_assertions)]
    let b = b.stack_size(64 << 20);
    b
}

fn connection(mut stream: std::net::TcpStream, handler: Handler, statics: &[Static]) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader, Read, Write};
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string());
    let mut headers = vec![];
    let mut length = 0;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            if k == "content-length" {
                length = v.parse().unwrap_or(0);
            }
            headers.push((k, v));
        }
    }
    let mut body = vec![0; length.min(64 << 20)];
    reader.read_exact(&mut body)?;
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let req = Request { method, path: path.into(), query: query.into(), headers, body: String::from_utf8_lossy(&body).into_owned() };
    let resp = respond(&req, handler, statics);
    let reason = match resp.status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let mut head = format!("HTTP/1.1 {} {reason}\r\n", resp.status);
    for (k, v) in &resp.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\nConnection: close\r\n\r\n", resp.body.len()));
    stream.write_all(head.as_bytes())?;
    stream.write_all(&resp.body)?;
    stream.flush()
}

fn respond(req: &Request, handler: Handler, statics: &[Static]) -> Response {
    if req.method == "GET"
        && let Some((_, ctype, text)) = statics.iter().find(|(p, _, _)| *p == req.path)
    {
        return Response { status: 200, headers: vec![("Content-Type".into(), ctype.to_string())], body: text.as_bytes().to_vec() };
    }
    handler(req).unwrap_or_else(|| error_response(404, &format!("no route {} {}", req.method, req.path)))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(x) => {
                    out.push(x);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
"##;

pub fn http_runtime(_n: i64) -> Result<Str, String> {
    Ok(HTTP_RUNTIME.into())
}

/// A server's name as part of a Rust identifier: `web-api` -> `web_api`.
pub fn server_ident(name: Str) -> Result<Str, String> {
    Ok(snake(cap_(&name).into())?
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .into())
}

fn cap_(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| {
        f.to_ascii_uppercase().to_string() + c.as_str()
    })
}

pub fn sort_strs(xs: List<Str>) -> Result<List<Str>, String> {
    let mut v = xs.to_vec();
    v.sort();
    v.dedup();
    Ok(v.into())
}

pub fn quote_twice(s: Str) -> Result<Str, String> {
    quote(quote(s)?)
}

pub fn comma(i: i64) -> Result<Str, String> {
    Ok(if i > 0 { ",".into() } else { "".into() })
}

pub fn find_top(prog: Program, name: Str) -> Result<i64, String> {
    Ok(prog
        .top
        .iter()
        .copied()
        .find(|&p| {
            prog.nodes[p as usize].qual.last() == Some(&name)
                && matches!(prog.nodes[p as usize].kind, NodeKind::Proc(_))
        })
        .unwrap_or(-1))
}

/// The procs routes of servers without a setup run (those with one get
/// calls of their own).
pub fn route_procs(servers: List<ServerSpec>) -> Result<List<Str>, String> {
    Ok(servers
        .iter()
        .filter(|s| s.setup.is_empty())
        .flat_map(|s| s.routes.iter().map(|r| r.proc.clone()).collect::<Vec<_>>())
        .collect())
}

pub fn server_of(servers: List<ServerSpec>, proc: Str) -> Result<Str, String> {
    Ok(servers
        .iter()
        .filter(|s| s.setup.is_empty())
        .find(|s| s.routes.iter().any(|r| r.proc == proc))
        .map_or("".into(), |s| s.name.clone()))
}

pub fn http_decodes(g: RGen, pid: i64) -> Result<List<Line>, String> {
    params_of(&g.prog.nodes, pid)
        .iter()
        .map(|m| {
            Ok(Line::L(
                format!(
                    "let {}: {} = arg(params, {})?;",
                    rust_local(m.name.clone())?,
                    rtype(&g.table, &m.ty)?,
                    q(&m.name)
                )
                .into(),
            ))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(List::from)
}

pub fn http_args(g: RGen, pid: i64) -> Result<Str, String> {
    Ok(params_of(&g.prog.nodes, pid)
        .iter()
        .map(|m| rust_local(m.name.clone()).map(|s| s.to_string()))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ")
        .into())
}

pub fn main_dir(module_dir: Str, path: Str) -> Result<RustServerMainDirOut, String> {
    let up = vec![".."; module_dir.split('/').filter(|x| !x.is_empty()).count()];
    let p = path.trim_matches('/');
    let (sub, file) = p.rsplit_once('/').unwrap_or(("", p));
    let dir = up
        .into_iter()
        .chain((!sub.is_empty()).then_some(sub))
        .collect::<Vec<_>>()
        .join("/");
    Ok(RustServerMainDirOut {
        dir: dir.into(),
        file: file.into(),
    })
}

pub fn replace_crate(path: Str, krate: Str) -> Result<Str, String> {
    Ok(path.replacen("crate", &krate, 1).into())
}

pub fn route_docs(srv: ServerSpec) -> Result<List<Line>, String> {
    let mut out: Vec<Line> = srv
        .routes
        .iter()
        .map(|r| Line::L(format!("//! - `{} {}` runs proc {}", r.method, r.path, r.proc).into()))
        .collect();
    out.extend(srv.raw.iter().map(|r| {
        Line::L(
            format!(
                "//! - `{} {}` runs proc {} on the raw request",
                r.method, r.path, r.proc
            )
            .into(),
        )
    }));
    out.extend(
        srv.statics
            .iter()
            .map(|s| Line::L(format!("//! - `GET {}` serves {}", s.url, s.file).into())),
    );
    if !srv.setup.is_empty() {
        out.push(Line::L("//!".into()));
        out.push(Line::L(
            format!(
                "//! Proc {} runs first; its result is each route's ctx.",
                srv.setup
            )
            .into(),
        ));
    }
    Ok(out.into())
}

const RAW_REQUEST: &[(&str, &str)] = &[
    ("method", "string"),
    ("path", "string"),
    ("query", "map[string]string"),
    ("headers", "map[string]string"),
    ("body", "string"),
];
const RAW_RESPONSE: &[(&str, &str)] = &[
    ("status", "int"),
    ("headers", "map[string]string"),
    ("body", "string"),
];

fn show_type(t: &Type) -> String {
    crate::vcc::run_show_type(t.clone())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Whether proc pid takes a raw request and returns a raw response; or the
/// error saying what it must.
pub fn raw_shape_error(
    prog: Program,
    pid: i64,
    server: Str,
    name: Str,
) -> Result<MaybeStr, String> {
    let params = params_of(&prog.nodes, pid);
    let ok = params.len() == 1
        && raw_shape(&prog, &params[0].ty, RAW_REQUEST)
        && raw_shape(&prog, &ret_of(&prog.nodes, pid), RAW_RESPONSE);
    Ok(if ok {
        MaybeStr::None
    } else {
        MaybeStr::Some(
            format!(
                "http server {server}: raw route proc {name} must take one {} and return {}",
                show_shape(RAW_REQUEST),
                show_shape(RAW_RESPONSE)
            )
            .into(),
        )
    })
}

/// Whether t is a struct of exactly these fields (a raw request or response).
fn raw_shape(prog: &Program, t: &Type, want: &[(&str, &str)]) -> bool {
    let defs = &prog.types;
    match unfold(t, defs) {
        Type::Struct(fs) => {
            fs.len() == want.len()
                && want.iter().all(|(n, w)| {
                    fs.iter().any(|f| {
                        &*f.name == *n
                            && (show_type(&f.ty) == *w || show_type(unfold(&f.ty, defs)) == *w)
                    })
                })
        }
        _ => false,
    }
}

fn show_shape(fs: &[(&str, &str)]) -> String {
    format!(
        "{{ {} }}",
        fs.iter()
            .map(|(n, t)| format!("{n}: {t}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// What a server's handler calls a route's proc through: its own calls
/// (`call_<server>_<proc>`) if it has a setup.
pub fn own_prefix(srv: ServerSpec) -> Result<Str, String> {
    Ok(if srv.setup.is_empty() {
        "".into()
    } else {
        format!("{}_", server_ident(srv.name)?).into()
    })
}

/// A server's main: run, or run_setup if it has a setup.
pub fn run_call(srv: ServerSpec, http: Str) -> Result<Str, String> {
    let ident = server_ident(srv.name.clone())?;
    Ok(if srv.setup.is_empty() {
        format!(
            "{http}::run({}, {http}::handle_{ident}, STATICS);",
            q(&srv.addr)
        )
    } else {
        format!(
            "{http}::run_setup({}, {http}::setup_{ident}, {http}::handle_{ident}, STATICS);",
            q(&srv.addr)
        )
    }
    .into())
}

/// A server with a setup proc, for each server that has one: its context,
/// setup_<server> to run the setup before serving, and a call per route
/// and raw route passing the context as ctx (never read from the request).
pub fn setup_calls(g: RGen, servers: List<ServerSpec>) -> Result<RustHttpSetupCallsOut, String> {
    let mut lines = vec![];
    for srv in servers.iter().filter(|s| !s.setup.is_empty()) {
        if let Err(e) = setup_server(&g, srv, &mut lines) {
            return Ok(RustHttpSetupCallsOut {
                lines: List::default(),
                err: MaybeStr::Some(e.into()),
            });
        }
    }
    Ok(RustHttpSetupCallsOut {
        lines: lines.into(),
        err: MaybeStr::None,
    })
}

fn setup_server(g: &RGen, srv: &ServerSpec, w: &mut Vec<Line>) -> Result<(), String> {
    let prog = &g.prog;
    let (name, setup) = (&*srv.name, &*srv.setup);
    let mut line = |s: String| w.push(Line::L(s.into()));
    let find = |p: &str| -> Result<i64, String> {
        match find_top(prog.clone(), p.into())? {
            -1 => Err(format!(
                "http server {name} runs proc {p}, but there is no top-level proc {p}"
            )),
            pid => Ok(pid),
        }
    };
    let sid = match find_top(prog.clone(), setup.into())? {
        -1 => {
            return Err(format!(
                "http server {name}'s setup runs proc {setup}, but there is no top-level proc {setup}"
            ));
        }
        sid => sid,
    };
    if !params_of(&prog.nodes, sid).is_empty() {
        return Err(format!(
            "http server {name}: setup proc {setup} must take no parameters"
        ));
    }
    let ctx_ty = ret_of(&prog.nodes, sid);
    let ident = server_ident(srv.name.clone())?;
    let (rctx, stat) = (
        rtype(&g.table, &ctx_ty)?,
        format!("CTX_{}", ident.to_uppercase()),
    );
    // Each of the server's procs must take ctx, of the setup's type.
    let check = |kind: &str, p: &str, pid: i64| -> Result<(), String> {
        match params_of(&prog.nodes, pid)
            .iter()
            .find(|m| &*m.name == "ctx")
        {
            None => Err(format!(
                "http server {name}: {kind} proc {p} must take ctx: {}, its setup proc {setup}'s result",
                show_type(&ctx_ty)
            )),
            Some(m) if rtype(&g.table, &m.ty)? != rctx => Err(format!(
                "http server {name}: {kind} proc {p} takes ctx: {}, but its setup proc {setup} returns {}",
                show_type(&m.ty),
                show_type(&ctx_ty)
            )),
            Some(_) => Ok(()),
        }
    };
    let sn = |pid: i64| snake_gname(prog.nodes[pid as usize].clone()).map(|s| s.to_string());
    line(format!(
        "/// The context of http server {name}: proc {setup}'s result, set before serving."
    ));
    line(format!(
        "static {stat}: std::sync::OnceLock<{rctx}> = std::sync::OnceLock::new();"
    ));
    line(String::new());
    line(format!(
        "/// Runs proc {setup}, the setup of http server {name}."
    ));
    line(format!("pub fn setup_{ident}() -> Result<(), String> {{"));
    line(format!(
        "let ctx = super::run_{}().map_err(|e| e.to_string())?;",
        sn(sid)?
    ));
    line(format!("let _ = {stat}.set(ctx);"));
    line("Ok(())".into());
    line("}".into());
    line(String::new());
    let unset = q(&format!("http server {name} is not set up"));
    let mut procs: Vec<&str> = srv.routes.iter().map(|r| &*r.proc).collect();
    procs.sort();
    procs.dedup();
    for p in procs {
        let pid = find(p)?;
        check("route", p, pid)?;
        let fname = sn(pid)?;
        line(format!(
            "/// Runs proc {p} with the request's parameters and http server {name}'s context."
        ));
        line(format!(
            "fn call_{ident}_{fname}(params: &Value) -> Result<String, (u16, String)> {{"
        ));
        let mut args = vec![];
        for m in params_of(&prog.nodes, pid).iter() {
            let var = rust_local(m.name.clone())?.to_string();
            if &*m.name == "ctx" {
                line(format!(
                    "let {var}: {rctx} = {stat}.get().cloned().ok_or_else(|| (500, {unset}.to_string()))?;"
                ));
            } else {
                line(format!(
                    "let {var}: {} = arg(params, {})?;",
                    rtype(&g.table, &m.ty)?,
                    q(&m.name)
                ));
            }
            args.push(var);
        }
        line(format!(
            "let result = super::run_{fname}({}).map_err(|e| (500, e.to_string()))?;",
            args.join(", ")
        ));
        line("Ok(json(&result))".into());
        line("}".into());
        line(String::new());
    }
    let mut raws: Vec<&str> = srv.raw.iter().map(|r| &*r.proc).collect();
    raws.sort();
    raws.dedup();
    for p in raws {
        let pid = find(p)?;
        check("raw route", p, pid)?;
        let params = params_of(&prog.nodes, pid);
        let rest: Vec<&Member> = params.iter().filter(|m| &*m.name != "ctx").collect();
        if !(rest.len() == 1
            && raw_shape(prog, &rest[0].ty, RAW_REQUEST)
            && raw_shape(prog, &ret_of(&prog.nodes, pid), RAW_RESPONSE))
        {
            return Err(format!(
                "http server {name}: raw route proc {p} must take ctx and one {}, and return {}",
                show_shape(RAW_REQUEST),
                show_shape(RAW_RESPONSE)
            ));
        }
        let fname = sn(pid)?;
        line(format!(
            "/// Runs proc {p} on the raw request with http server {name}'s context."
        ));
        line(format!(
            "fn raw_{ident}_{fname}(req: &Request) -> Response {{"
        ));
        line(format!("let r = {} {{", rtype(&g.table, &rest[0].ty)?));
        line("method: types::Str::from(req.method.as_str()),".into());
        line("path: types::Str::from(req.path.as_str()),".into());
        line("query: query_pairs(&req.query).into_iter().map(|(k, v)| (types::Str::from(k.as_str()), types::Str::from(v.as_str()))).collect(),".into());
        line("headers: req.headers.iter().map(|(k, v)| (types::Str::from(k.as_str()), types::Str::from(v.as_str()))).collect(),".into());
        line("body: types::Str::from(req.body.as_str()),".into());
        line("};".into());
        line(format!("let ctx = match {stat}.get() {{"));
        line("Some(c) => c.clone(),".into());
        line(format!("None => return error_response(500, {unset}),"));
        line("};".into());
        let args: Vec<&str> = params
            .iter()
            .map(|m| if &*m.name == "ctx" { "ctx" } else { "r" })
            .collect();
        line(format!("match super::run_{fname}({}) {{", args.join(", ")));
        line("Ok(res) => Response {".into());
        line("status: res.status as u16,".into());
        line(
            "headers: res.headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),"
                .into(),
        );
        line("body: res.body.as_bytes().to_vec(),".into());
        line("},".into());
        line("Err(e) => error_response(500, &e.to_string()),".into());
        line("}".into());
        line("}".into());
        line(String::new());
    }
    Ok(())
}

pub fn raw_procs(servers: List<ServerSpec>) -> Result<List<Str>, String> {
    Ok(servers
        .iter()
        .filter(|s| s.setup.is_empty())
        .flat_map(|s| s.raw.iter().map(|r| r.proc.clone()).collect::<Vec<_>>())
        .collect())
}

pub fn raw_server_of(servers: List<ServerSpec>, proc: Str) -> Result<Str, String> {
    Ok(servers
        .iter()
        .filter(|s| s.setup.is_empty())
        .find(|s| s.raw.iter().any(|r| r.proc == proc))
        .map_or("".into(), |s| s.name.clone()))
}

pub fn raw_param_type(g: RGen, pid: i64) -> Result<Str, String> {
    Ok(rtype(&g.table, &params_of(&g.prog.nodes, pid)[0].ty)?.into())
}

/// The signature an implementation must have.
pub fn rust_signature(
    prog: Program,
    t: Table,
    n: i64,
    name: Str,
    types_path: Str,
) -> Result<Str, String> {
    let qual = format!("{types_path}::");
    let params: Vec<String> = params_of(&prog.nodes, n)
        .iter()
        .map(|m| {
            Ok(format!(
                "{}: {}",
                rust_local(m.name.clone())?,
                rust_render(
                    t.clone(),
                    get(t.clone(), m.ty.clone())?,
                    qual.as_str().into()
                )?
            ))
        })
        .collect::<Result<_, String>>()?;
    let ret = rust_render(
        t.clone(),
        get(t.clone(), ret_of(&prog.nodes, n))?,
        qual.as_str().into(),
    )?;
    Ok(format!("pub fn {name}({}) -> Result<{ret}, E>", params.join(", ")).into())
}

pub fn rs_stem(file: Str) -> Result<Str, String> {
    Ok(file
        .rsplit('/')
        .next()
        .unwrap_or("")
        .strip_suffix(".rs")
        .unwrap_or("")
        .into())
}

pub fn mountable_stem(file: Str) -> Result<bool, String> {
    let stem = rs_stem(file)?;
    let valid = !stem.is_empty()
        && stem
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()));
    Ok(valid && !["types", "procs", "http"].contains(&&*stem))
}

pub fn rs_impl_file(n: NodeDecl) -> Result<Str, String> {
    match rs_impl(n, "rs".into())? {
        MaybeImpl::Some(i) => Ok(i.file),
        MaybeImpl::None => Err("rs_impl_file: no .rs implementation".into()),
    }
}

pub fn mount_lines(mounts: Map<Str, Str>) -> Result<Str, String> {
    Ok(mounts
        .iter()
        .map(|(name, path)| format!("#[path = {}]\nmod {};\n", q(path), rust_ident_of(name)))
        .collect::<String>()
        .into())
}
