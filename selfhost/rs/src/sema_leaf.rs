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

// ---- Typedefs ----



const PRIMS: &[&str] = &["void", "int", "float", "bool", "string"];

pub fn is_prim(name: Str) -> Result<bool, String> {
    Ok(PRIMS.contains(&&*name))
}

pub fn itoa(i: i64) -> Result<Str, String> {
    Ok(i.to_string().into())
}

pub fn cap(s: Str) -> Result<Str, String> {
    let mut c = s.chars();
    Ok(c.next().map_or(String::new(), |f| f.to_ascii_uppercase().to_string() + c.as_str()).into())
}

pub fn plural(n: i64) -> Result<Str, String> {
    Ok(if n == 1 { "".into() } else { "s".into() })
}

pub fn join(parts: List<Str>, sep: Str) -> Result<Str, String> {
    Ok(parts.join(&*sep).into())
}

pub fn with_error(mut cx: TyCx, msg: Str, span: Span) -> Result<TyCx, String> {
    cx.errors = cx.errors.concat(&vec![Diag { msg, span }].into());
    Ok(cx)
}

pub fn with_errors(mut cx: TyCx, errors: List<Diag>) -> Result<TyCx, String> {
    cx.errors = errors;
    Ok(cx)
}

pub fn with_types(mut cx: TyCx, types: List<TypeDef>) -> Result<TyCx, String> {
    cx.types = types;
    Ok(cx)
}

pub fn with_bound(mut cx: TyCx, bound: List<Map<Str, Type>>) -> Result<TyCx, String> {
    cx.bound = bound;
    Ok(cx)
}

pub fn with_generic(mut cx: TyCx, d: TypeDecl) -> Result<TyCx, String> {
    cx.generics = cx.generics.merge(&Map::from_iter([(d.name.name.clone(), d)]));
    Ok(cx)
}

pub fn with_broken(mut cx: TyCx, name: Str) -> Result<TyCx, String> {
    cx.broken = cx.broken.merge(&Map::from_iter([(name, true)]));
    Ok(cx)
}

pub fn with_recursive(mut cx: TyCx, id: i64, rec: bool) -> Result<TyCx, String> {
    let mut v: Vec<TypeDef> = cx.types.into_iter().collect();
    v[id as usize].recursive = rec;
    cx.types = v.into();
    Ok(cx)
}

fn push_def(mut cx: TyCx, name: Str, ident: Str, span: Span) -> TyCx {
    cx.types = cx.types.concat(&vec![TypeDef { name, ident, ty: Type::Prim("void".into()), recursive: false }].into());
    cx.spans = cx.spans.concat(&vec![span].into());
    cx
}

pub fn new_def(cx: TyCx, name: Str, ident: Str, span: Span) -> Result<TyCx, String> {
    let id = cx.types.len() as i64;
    let mut cx = push_def(cx, name.clone(), ident, span);
    cx.names = cx.names.merge(&Map::from_iter([(name, id)]));
    Ok(cx)
}

pub fn new_inst(cx: TyCx, key: Str, ident: Str, span: Span) -> Result<TyCx, String> {
    let id = cx.types.len() as i64;
    let mut cx = push_def(cx, key.clone(), ident, span);
    cx.insts = cx.insts.merge(&Map::from_iter([(key, id)]));
    Ok(cx)
}

pub fn set_ty(types: List<TypeDef>, id: i64, t: Type) -> Result<List<TypeDef>, String> {
    let mut v: Vec<TypeDef> = types.into_iter().collect();
    v[id as usize].ty = t;
    Ok(v.into())
}

pub fn in_instance(errors: List<Diag>, from: i64, key: Str) -> Result<List<Diag>, String> {
    Ok(errors
        .into_iter()
        .enumerate()
        .map(|(i, mut e)| {
            if i as i64 >= from {
                e.msg = format!("{} (in {key})", e.msg).into();
            }
            e
        })
        .collect())
}

pub fn bound(bound: List<Map<Str, Type>>, name: Str) -> Result<MaybeType, String> {
    Ok(bound.last().and_then(|b| b.get(&name)).map_or(MaybeType::None, |t| MaybeType::Some(t.clone())))
}

pub fn is_bound(bound: List<Map<Str, Type>>, name: Str) -> Result<bool, String> {
    Ok(bound.last().is_some_and(|b| b.contains_key(&name)))
}

pub fn param_map(params: List<Ident>, args: List<Type>) -> Result<Map<Str, Type>, String> {
    Ok(params.iter().map(|p| p.name.clone()).zip(args.iter().cloned()).collect())
}

pub fn param_names(ps: List<Ident>) -> Result<Str, String> {
    Ok(ps.iter().map(|p| &*p.name).collect::<Vec<_>>().join(", ").into())
}

pub fn ident_names(ids: List<Ident>) -> Result<List<Str>, String> {
    Ok(ids.iter().map(|p| p.name.clone()).collect())
}

pub fn is_key_type(t: Type) -> Result<bool, String> {
    Ok(matches!(&t, Type::Prim(p) if &**p == "int" || &**p == "string"))
}

pub fn has_member(ms: List<Member>, name: Str) -> Result<bool, String> {
    Ok(ms.iter().any(|m| m.name == name))
}

pub fn has_int(seen: List<i64>, x: i64) -> Result<bool, String> {
    Ok(seen.contains(&x))
}

pub fn has_str(xs: List<Str>, x: Str) -> Result<bool, String> {
    Ok(xs.contains(&x))
}

pub fn is_struct_or_enum(t: Type) -> Result<bool, String> {
    Ok(matches!(t, Type::Struct(_) | Type::Enum(_)))
}

pub fn is_some(t: MaybeType) -> Result<bool, String> {
    Ok(matches!(t, MaybeType::Some(_)))
}

pub fn unwrap_t(t: MaybeType) -> Result<Type, String> {
    match t {
        MaybeType::Some(t) => Ok(t),
        MaybeType::None => Err("unwrap_t: None".into()),
    }
}

pub fn unwrap_tys(m: MaybeTypes) -> Result<List<Type>, String> {
    match m {
        MaybeTypes::Some(t) => Ok(t),
        MaybeTypes::None => Err("unwrap_tys: None".into()),
    }
}

pub fn array_of(t: MaybeType) -> Result<MaybeType, String> {
    Ok(match t {
        MaybeType::Some(t) => MaybeType::Some(Type::Array(std::sync::Arc::new(t))),
        MaybeType::None => MaybeType::None,
    })
}

pub fn struct_of(ms: MaybeMembers) -> Result<MaybeType, String> {
    Ok(match ms {
        MaybeMembers::Some(ms) => MaybeType::Some(Type::Struct(ms)),
        MaybeMembers::None => MaybeType::None,
    })
}

pub fn enum_of(ms: MaybeMembers) -> Result<MaybeType, String> {
    Ok(match ms {
        MaybeMembers::Some(ms) => MaybeType::Some(Type::Enum(ms)),
        MaybeMembers::None => MaybeType::None,
    })
}

pub fn map_of(k: Type, v: MaybeType) -> Result<MaybeType, String> {
    Ok(match v {
        MaybeType::Some(v) => MaybeType::Some(Type::Map(std::sync::Arc::new(TypeMap { key: std::sync::Arc::new(k), value: std::sync::Arc::new(v) }))),
        MaybeType::None => MaybeType::None,
    })
}

pub fn members_if(ok: bool, ms: List<Member>) -> Result<MaybeMembers, String> {
    Ok(if ok { MaybeMembers::Some(ms) } else { MaybeMembers::None })
}

pub fn map_value(t: Type) -> Result<Type, String> {
    match t {
        Type::Map(m) => Ok((*m.value).clone()),
        _ => Err("map_value: not a map".into()),
    }
}

pub fn field_types(fs: List<Field>) -> Result<List<TypeExpr>, String> {
    Ok(fs.iter().map(|f| f.ty.clone()).collect())
}

pub fn payload_types(vs: List<Variant>) -> Result<List<TypeExpr>, String> {
    Ok(vs.iter().filter_map(|v| match &v.payload {
        OptType::Some(p) => Some(p.clone()),
        OptType::None => None,
    }).collect())
}

pub fn name_span(k: TypeKind) -> Result<Span, String> {
    match k {
        TypeKind::Named(n) => Ok(n.span),
        TypeKind::Inst(i) => Ok(i.name.span),
        _ => Err("name_span: not a name".into()),
    }
}

pub fn inst_args(k: TypeKind) -> Result<List<TypeExpr>, String> {
    match k {
        TypeKind::Inst(i) => Ok(i.args),
        _ => Err("inst_args: not an instance".into()),
    }
}

/// A type's direct parts, in order (what NamedIn looks inside).
pub fn parts(t: Type) -> Result<List<Type>, String> {
    Ok(match t {
        Type::Prim(_) | Type::Named(_) => List::default(),
        Type::Array(e) => vec![(*e).clone()].into(),
        Type::Map(m) => vec![(*m.key).clone(), (*m.value).clone()].into(),
        Type::Struct(ms) | Type::Enum(ms) => ms.iter().map(|m| m.ty.clone()).collect(),
    })
}

pub fn name_of(t: Type) -> Result<List<i64>, String> {
    Ok(match t {
        Type::Named(n) => vec![n.id].into(),
        _ => List::default(),
    })
}

pub fn rest(ts: List<Type>) -> Result<List<Type>, String> {
    Ok(ts[1..].into())
}

pub fn last_int(xs: List<i64>) -> Result<i64, String> {
    xs.last().copied().ok_or_else(|| "last_int: empty".into())
}

pub fn pop_int(xs: List<i64>) -> Result<List<i64>, String> {
    Ok(xs[..xs.len() - 1].into())
}

// ---- Declarations ----

fn push<T: Clone>(xs: List<T>, x: T) -> List<T> {
    xs.concat(&vec![x].into())
}

fn updated<T: Clone>(xs: List<T>, i: i64, f: impl FnOnce(&mut T)) -> List<T> {
    let mut v: Vec<T> = xs.into_iter().collect();
    f(&mut v[i as usize]);
    v.into()
}

pub fn cx_error(mut cx: Cx, msg: Str, span: Span) -> Result<Cx, String> {
    cx.ty = with_error(cx.ty, msg, span)?;
    Ok(cx)
}

pub fn with_ty(mut cx: Cx, ty: TyCx) -> Result<Cx, String> {
    cx.ty = ty;
    Ok(cx)
}

pub fn with_node(mut cx: Cx, n: NodeDecl) -> Result<Cx, String> {
    cx.nodes = push(cx.nodes, n);
    Ok(cx)
}

pub fn with_global(mut cx: Cx, name: Str, g: Global) -> Result<Cx, String> {
    cx.globals = cx.globals.merge(&Map::from_iter([(name, g)]));
    Ok(cx)
}

pub fn with_top(mut cx: Cx, id: i64) -> Result<Cx, String> {
    cx.top = push(cx.top, id);
    Ok(cx)
}

pub fn with_id(mut cx: Cx, at: i64, id: i64) -> Result<Cx, String> {
    cx.ids = cx.ids.merge(&Map::from_iter([(at, id)]));
    Ok(cx)
}

pub fn with_alias(mut cx: Cx, a: PendingAlias) -> Result<Cx, String> {
    cx.aliases = push(cx.aliases, a);
    Ok(cx)
}

pub fn with_proc(mut cx: Cx, p: PendingProc) -> Result<Cx, String> {
    cx.procs = push(cx.procs, p);
    Ok(cx)
}

pub fn set_proc(mut cx: Cx, id: i64, entry: i64, locals: List<i64>) -> Result<Cx, String> {
    cx.nodes = updated(cx.nodes, id, |n| {
        if let NodeKind::Proc(p) = &mut n.kind {
            p.entry = entry;
            p.locals = locals;
        }
    });
    Ok(cx)
}

pub fn set_alias(mut cx: Cx, lid: i64, of: i64) -> Result<Cx, String> {
    cx.nodes = updated(cx.nodes, lid, |n| n.kind = NodeKind::Alias(of));
    Ok(cx)
}

pub fn empty_graph(_n: i64) -> Result<Graph, String> {
    Ok(Graph { edges: Map::default(), regions: List::default(), region_of: Map::default(), opens: Map::default() })
}

pub fn show_path(p: Path) -> Result<Str, String> {
    Ok(p.segs.iter().map(|s| &*s.name).collect::<Vec<_>>().join(".").into())
}

pub fn qual_name(n: NodeDecl) -> Result<Str, String> {
    Ok(n.qual.iter().map(|s| &**s).collect::<Vec<_>>().join(".").into())
}

pub fn proc_ret(n: NodeDecl) -> Result<Type, String> {
    match n.kind {
        NodeKind::Proc(p) => Ok(p.ret),
        _ => Err("proc_ret: not a proc".into()),
    }
}

pub fn is_alias(n: NodeDecl) -> Result<bool, String> {
    Ok(matches!(n.kind, NodeKind::Alias(_)))
}

pub fn node_name(n: Node) -> Result<Ident, String> {
    Ok(match n {
        Node::Func(f) => f.name,
        Node::Proc(p) => p.name,
    })
}

pub fn top_arms(stack: List<List<Arm>>) -> Result<List<Arm>, String> {
    stack.last().cloned().ok_or_else(|| "top_arms: empty".into())
}

pub fn pop_arms(stack: List<List<Arm>>) -> Result<List<List<Arm>>, String> {
    Ok(stack[..stack.len() - 1].into())
}

pub fn rest_arms(arms: List<Arm>) -> Result<List<Arm>, String> {
    Ok(arms[1..].into())
}

pub fn is_id(t: Target) -> Result<bool, String> {
    Ok(matches!(t.id, OptValue::Some(_)))
}

pub fn alias_of(t: Target) -> Result<OptIdent, String> {
    Ok(t.alias)
}

/// The local called `name` of proc `pid` or an enclosing proc, innermost
/// first; -1 if none.
pub fn find_local(nodes: List<NodeDecl>, pid: i64, name: Str) -> Result<i64, String> {
    let mut scope = pid;
    while scope >= 0 {
        let NodeKind::Proc(p) = &nodes[scope as usize].kind else { return Err("find_local: not a proc".into()) };
        if let Some(&l) = p.locals.iter().find(|&&l| nodes[l as usize].qual.last() == Some(&name)) {
            return Ok(l);
        }
        scope = p.parent;
    }
    Ok(-1)
}

pub fn segs_from(ids: List<Ident>, from: i64) -> Result<List<Ident>, String> {
    Ok(ids[(from as usize).min(ids.len())..].into())
}
