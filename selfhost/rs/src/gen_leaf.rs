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
