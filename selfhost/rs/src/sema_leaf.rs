//! Leaves for vcc's sema (../sys/sema): small pure helpers. The analysis
//! itself is in .sys.

use crate::vcc::types::*;

/// The identity, for any type: entries and pass-through nodes of procs that
/// take one struct (`func At(s: S): S`).
pub fn same<T>(x: T) -> Result<T, String> {
    Ok(x)
}

/// `", "` between list items: nothing before the first.
pub fn sep(i: i64) -> Result<Str, String> {
    Ok(if i == 0 { "".into() } else { ", ".into() })
}

pub fn has_pair(ps: List<Pair>, p: Pair) -> Result<bool, String> {
    Ok(ps.contains(&p))
}

/// Both typedef ids, if both types are typedef names.
pub fn named_pair(a: Type, b: Type) -> Result<OptPair, String> {
    Ok(match (&a, &b) {
        (Type::Named(x), Type::Named(y)) => OptPair::Some(Pair { x: x.id, y: y.id }),
        _ => OptPair::None,
    })
}

pub fn is_void(t: Type) -> Result<bool, String> {
    Ok(matches!(&t, Type::Prim(p) if &**p == "void"))
}

/// The member named `name`, if any.
pub fn member(ms: List<Member>, name: Str) -> Result<MaybeType, String> {
    Ok(ms.iter().find(|m| m.name == name).map_or(MaybeType::None, |m| MaybeType::Some(m.ty.clone())))
}


pub fn new_pair(o: OptPair) -> Result<List<Pair>, String> {
    Ok(match o {
        OptPair::Some(p) => vec![p].into(),
        OptPair::None => List::default(),
    })
}
