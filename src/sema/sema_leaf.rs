//! Leaves for vcc's sema (the .sys files here): small pure helpers. The analysis
//! itself is in .sys.

use crate::vcc::types::*;

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
    Ok(ms
        .iter()
        .find(|m| m.name == name)
        .map_or(MaybeType::None, |m| MaybeType::Some(m.ty.clone())))
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
    Ok(c.next()
        .map_or(String::new(), |f| {
            f.to_ascii_uppercase().to_string() + c.as_str()
        })
        .into())
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

pub fn with_recursive(mut cx: TyCx, id: i64, rec: bool) -> Result<TyCx, String> {
    let mut v: Vec<TypeDef> = cx.types.into_iter().collect();
    v[id as usize].recursive = rec;
    cx.types = v.into();
    Ok(cx)
}

fn push_def(mut cx: TyCx, name: Str, ident: Str, span: Span) -> TyCx {
    cx.types = cx.types.concat(
        &vec![TypeDef {
            name,
            ident,
            ty: Type::Prim("void".into()),
            recursive: false,
        }]
        .into(),
    );
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
    Ok(bound
        .last()
        .and_then(|b| b.get(&name))
        .map_or(MaybeType::None, |t| MaybeType::Some(t.clone())))
}

pub fn is_bound(bound: List<Map<Str, Type>>, name: Str) -> Result<bool, String> {
    Ok(bound.last().is_some_and(|b| b.contains_key(&name)))
}

pub fn param_map(params: List<Ident>, args: List<Type>) -> Result<Map<Str, Type>, String> {
    Ok(params
        .iter()
        .map(|p| p.name.clone())
        .zip(args.iter().cloned())
        .collect())
}

pub fn param_names(ps: List<Ident>) -> Result<Str, String> {
    Ok(ps
        .iter()
        .map(|p| &*p.name)
        .collect::<Vec<_>>()
        .join(", ")
        .into())
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
        MaybeType::Some(v) => MaybeType::Some(Type::Map(std::sync::Arc::new(TypeMap {
            key: std::sync::Arc::new(k),
            value: std::sync::Arc::new(v),
        }))),
        MaybeType::None => MaybeType::None,
    })
}

pub fn members_if(ok: bool, ms: List<Member>) -> Result<MaybeMembers, String> {
    Ok(if ok {
        MaybeMembers::Some(ms)
    } else {
        MaybeMembers::None
    })
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
    Ok(vs
        .iter()
        .filter_map(|v| match &v.payload {
            OptType::Some(p) => Some(p.clone()),
            OptType::None => None,
        })
        .collect())
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

pub fn with_node(mut cx: Cx, n: NodeDecl) -> Result<Cx, String> {
    cx.nodes = push(cx.nodes, n);
    Ok(cx)
}

pub fn span_key(s: Span) -> Result<Str, String> {
    Ok(format!("{}:{}", s.file, s.at).into())
}

pub fn with_id(mut cx: Cx, at: Str, id: i64) -> Result<Cx, String> {
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

/// cx with proc `pid`'s signature node after it, if it has parameters
/// rather than a named entry: a local of the proc with the proc's name
/// (`P.P`), so `P` in the body names it.
pub fn with_sig(
    mut cx: Cx,
    p: Proc,
    qual: List<Str>,
    pid: i64,
    params: List<Member>,
) -> Result<Cx, String> {
    if let OptIdent::None = p.entry {
        cx.nodes = push(
            cx.nodes,
            NodeDecl {
                qual: push(qual, p.name.name.clone()),
                span: p.name.span,
                kind: NodeKind::Sig(NodeKindSig { pid, params }),
            },
        );
    }
    Ok(cx)
}

/// The proc's own locals before its declared ones: its signature node.
pub fn sig_names(p: Proc, pid: i64) -> Result<Map<Str, i64>, String> {
    Ok(match p.entry {
        OptIdent::None => Map::from_iter([(p.name.name, pid + 1)]),
        OptIdent::Some(_) => Map::default(),
    })
}

pub fn sig_locals(p: Proc, pid: i64) -> Result<List<i64>, String> {
    Ok(match p.entry {
        OptIdent::None => vec![pid + 1].into(),
        OptIdent::Some(_) => List::default(),
    })
}

/// The signature node, or the named entry's local (-1 if there's none).
pub fn entry_id(p: Proc, pid: i64, names: Map<Str, i64>) -> Result<i64, String> {
    Ok(match p.entry {
        OptIdent::None => pid + 1,
        OptIdent::Some(e) => names.get(&e.name).copied().unwrap_or(-1),
    })
}

pub fn entry_ident(p: Proc) -> Result<Ident, String> {
    Ok(match p.entry {
        OptIdent::Some(e) => e,
        OptIdent::None => p.name,
    })
}

/// The proc a signature node belongs to; any other node is itself.
pub fn sig_proc(n: NodeDecl, b: i64) -> Result<i64, String> {
    Ok(match n.kind {
        NodeKind::Sig(s) => s.pid,
        _ => b,
    })
}

/// A call target: the signature node of another proc (an enclosing one)
/// calls that proc; in its own proc it's the back edge to the entry.
pub fn call_target(nodes: List<NodeDecl>, pid: i64, n: i64) -> Result<i64, String> {
    Ok(match &nodes[n as usize].kind {
        NodeKind::Sig(s) if s.pid != pid => s.pid,
        _ => n,
    })
}

/// `id(..) as N` before its type is known.
pub fn is_untyped_id(n: NodeDecl) -> Result<bool, String> {
    Ok(matches!(&n.kind, NodeKind::Id(ps) if ps.is_empty()))
}

/// nodes with id node `n`'s value typed `t`.
pub fn with_id_type(nodes: List<NodeDecl>, n: i64, t: Type) -> Result<List<NodeDecl>, String> {
    Ok(updated(nodes, n, |d| {
        d.kind = NodeKind::Id(
            vec![Member {
                name: "value".into(),
                ty: t,
            }]
            .into(),
        )
    }))
}

pub fn set_alias(mut cx: Cx, lid: i64, of: i64) -> Result<Cx, String> {
    cx.nodes = updated(cx.nodes, lid, |n| n.kind = NodeKind::Alias(of));
    Ok(cx)
}

pub fn empty_graph(_n: i64) -> Result<Graph, String> {
    Ok(Graph {
        edges: Map::default(),
        regions: List::default(),
        region_of: Map::default(),
        opens: Map::default(),
        spawned: Map::default(),
    })
}

pub fn show_path(p: Path) -> Result<Str, String> {
    Ok(p.segs
        .iter()
        .map(|s| &*s.name)
        .collect::<Vec<_>>()
        .join(".")
        .into())
}

pub fn qual_name(n: NodeDecl) -> Result<Str, String> {
    Ok(n.qual
        .iter()
        .map(|s| &**s)
        .collect::<Vec<_>>()
        .join(".")
        .into())
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
    stack
        .last()
        .cloned()
        .ok_or_else(|| "top_arms: empty".into())
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
        let NodeKind::Proc(p) = &nodes[scope as usize].kind else {
            return Err("find_local: not a proc".into());
        };
        if let Some(&l) = p
            .locals
            .iter()
            .find(|&&l| nodes[l as usize].qual.last() == Some(&name))
        {
            return Ok(l);
        }
        scope = p.parent;
    }
    Ok(-1)
}

pub fn segs_from(ids: List<Ident>, from: i64) -> Result<List<Ident>, String> {
    Ok(ids[(from as usize).min(ids.len())..].into())
}

// ---- Expressions ----

fn err<T>(what: &str) -> Result<T, String> {
    Err(format!("sema leaf: {what}"))
}

pub fn int_to_float(i: i64) -> Result<f64, String> {
    Ok(i as f64)
}

pub fn is_prim_named(t: Type, p: Str) -> Result<bool, String> {
    Ok(matches!(&t, Type::Prim(q) if *q == p))
}

pub fn prim_name(t: Type) -> Result<Str, String> {
    Ok(match t {
        Type::Prim(p) => p,
        _ => "".into(),
    })
}

pub fn is_array(t: Type) -> Result<bool, String> {
    Ok(matches!(t, Type::Array(_)))
}

pub fn is_map(t: Type) -> Result<bool, String> {
    Ok(matches!(t, Type::Map(_)))
}

pub fn is_enum(t: Type) -> Result<bool, String> {
    Ok(matches!(t, Type::Enum(_)))
}

pub fn is_struct(t: Type) -> Result<bool, String> {
    Ok(matches!(t, Type::Struct(_)))
}

pub fn members_of(t: Type) -> Result<List<Member>, String> {
    Ok(match t {
        Type::Struct(ms) | Type::Enum(ms) => ms,
        _ => List::default(),
    })
}

pub fn elem_of(t: Type) -> Result<Type, String> {
    match t {
        Type::Array(e) => Ok((*e).clone()),
        _ => err("elem_of"),
    }
}

pub fn key_of(t: Type) -> Result<Type, String> {
    match t {
        Type::Map(m) => Ok((*m.key).clone()),
        _ => err("key_of"),
    }
}

pub fn indexed_value(t: Type) -> Result<Type, String> {
    match t {
        Type::Map(m) => Ok((*m.value).clone()),
        Type::Array(e) => Ok((*e).clone()),
        _ => err("indexed_value"),
    }
}

pub fn lit_key(e: Expr) -> Result<MaybeStr, String> {
    Ok(match e {
        Expr::Int(i) => MaybeStr::Some(i.to_string().into()),
        Expr::Str(s) => MaybeStr::Some(format!("{:?}", &*s).into()),
        _ => MaybeStr::None,
    })
}

pub fn some_strs(m: MaybeStr) -> Result<List<Str>, String> {
    Ok(match m {
        MaybeStr::Some(s) => vec![s].into(),
        MaybeStr::None => List::default(),
    })
}

pub fn node_ret(nodes: List<NodeDecl>, n: i64) -> Result<Type, String> {
    match &nodes[n as usize].kind {
        NodeKind::Func(f) => Ok(f.ret.clone()),
        NodeKind::Proc(p) => Ok(p.ret.clone()),
        NodeKind::Alias(b) => node_ret(nodes.clone(), *b),
        NodeKind::Id(ps) => match ps.first() {
            Some(m) => Ok(m.ty.clone()),
            None => err("node_ret: an id whose type isn't known"),
        },
        NodeKind::Sig(s) => Ok(Type::Struct(s.params.clone())),
    }
}

pub fn node_params(nodes: List<NodeDecl>, n: i64) -> Result<List<Member>, String> {
    match &nodes[n as usize].kind {
        NodeKind::Func(f) => Ok(f.params.clone()),
        NodeKind::Proc(p) => node_params(nodes.clone(), p.entry),
        NodeKind::Alias(b) => node_params(nodes.clone(), *b),
        NodeKind::Id(ps) => Ok(ps.clone()),
        NodeKind::Sig(s) => Ok(s.params.clone()),
    }
}

pub fn is_func(n: NodeDecl) -> Result<bool, String> {
    Ok(matches!(n.kind, NodeKind::Func(_)))
}

pub fn prefixed(mut d: Diag, prefix: Str) -> Result<Diag, String> {
    d.msg = format!("{prefix}{}", d.msg).into();
    Ok(d)
}

pub fn binary_r(k: ValueKind) -> Result<Value, String> {
    match k {
        ValueKind::Binary(b) => Ok((*b.r).clone()),
        _ => err("binary_r"),
    }
}

pub fn binary_op(k: ValueKind) -> Result<Str, String> {
    match k {
        ValueKind::Binary(b) => Ok(b.op.clone()),
        _ => err("binary_op"),
    }
}

pub fn apply_name(k: ValueKind) -> Result<Path, String> {
    match k {
        ValueKind::Apply(a) => Ok(a.name),
        _ => err("apply_name"),
    }
}

pub fn field_name(k: ValueKind) -> Result<Ident, String> {
    match k {
        ValueKind::Field(f) => Ok(f.name.clone()),
        _ => err("field_name"),
    }
}

pub fn index_key(k: ValueKind) -> Result<Value, String> {
    match k {
        ValueKind::Index(x) => Ok((*x.key).clone()),
        _ => err("index_key"),
    }
}

pub fn index_of(k: ValueKind) -> Result<Value, String> {
    match k {
        ValueKind::Index(x) => Ok((*x.e).clone()),
        _ => err("index_of"),
    }
}

pub fn in_key(k: ValueKind) -> Result<Value, String> {
    match k {
        ValueKind::In(x) => Ok((*x.key).clone()),
        _ => err("in_key"),
    }
}

pub fn in_map(k: ValueKind) -> Result<Value, String> {
    match k {
        ValueKind::In(x) => Ok((*x.map).clone()),
        _ => err("in_map"),
    }
}

pub fn unary_operand(k: ValueKind) -> Result<Value, String> {
    match k {
        ValueKind::Unary(u) => Ok((*u.e).clone()),
        _ => err("unary_operand"),
    }
}

pub fn unwrap_e(r: ResExpr) -> Result<Expr, String> {
    match r {
        ResExpr::Ok(e) => Ok(e),
        ResExpr::Err(_) => err("unwrap_e"),
    }
}

pub fn unwrap_typed(r: ResTyped) -> Result<Typed, String> {
    match r {
        ResTyped::Ok(t) => Ok(t),
        ResTyped::Err(_) => err("unwrap_typed"),
    }
}

pub fn unwrap_n(r: ResolvedNode) -> Result<i64, String> {
    match r {
        ResolvedNode::Ok(n) => Ok(n),
        ResolvedNode::Err(_) => err("unwrap_n"),
    }
}

pub fn article(k: ValueKind) -> Result<Str, String> {
    Ok(if matches!(k, ValueKind::Int(_)) {
        "an".into()
    } else {
        "a".into()
    })
}

pub fn what(v: Value) -> Result<Str, String> {
    match &*v.kind {
        ValueKind::Ref(p) => show_path(p.clone()),
        _ => Ok("this expression".into()),
    }
}

pub fn first_arg_name(args: List<ApplyArg>) -> Result<OptIdent, String> {
    Ok(args
        .iter()
        .find_map(|a| match &a.name {
            OptIdent::Some(n) => Some(OptIdent::Some(n.clone())),
            OptIdent::None => None,
        })
        .unwrap_or(OptIdent::None))
}

pub fn named_args(args: List<ApplyArg>) -> Result<MaybeArgs, String> {
    let mut out = vec![];
    for a in args.iter() {
        match &a.name {
            OptIdent::Some(n) => out.push(Arg {
                name: n.clone(),
                value: a.value.clone(),
            }),
            OptIdent::None => return Ok(MaybeArgs::None(a.value.span.clone())),
        }
    }
    Ok(MaybeArgs::Some(out.into()))
}

pub fn id_value(t: Target) -> Result<OptValue, String> {
    Ok(t.id)
}

pub fn own_local(nodes: List<NodeDecl>, pid: i64, name: Str) -> Result<i64, String> {
    let NodeKind::Proc(p) = &nodes[pid as usize].kind else {
        return err("own_local");
    };
    p.locals
        .iter()
        .copied()
        .find(|&l| nodes[l as usize].qual.last() == Some(&name))
        .ok_or_else(|| "own_local: missing".into())
}

fn allowed_prims(op: &str) -> &'static [&'static str] {
    match op {
        "+" => &["int", "float", "string"],
        "-" | "*" | "/" => &["int", "float"],
        "%" => &["int"],
        "<" | "<=" | ">" | ">=" => &["int", "float", "string"],
        _ => &["int", "float", "string", "bool"],
    }
}

pub fn allowed(op: Str) -> Result<Str, String> {
    Ok(allowed_prims(&op).join(", ").into())
}

pub fn operands(s: OperandsS) -> Result<OperandsOut, String> {
    let (mut l, mut r, mut lp, mut rp) = (s.l, s.r, s.lp, s.rp);
    // An int literal next to a float is a float.
    if let (Expr::Int(i), "int", "float") = (&l, &*lp, &*rp) {
        l = Expr::Float(*i as f64);
        lp = "float".into();
    }
    if let (Expr::Int(i), "float", "int") = (&r, &*lp, &*rp) {
        r = Expr::Float(*i as f64);
        rp = "float".into();
    }
    let ok = !lp.is_empty() && lp == rp && allowed_prims(&s.op).contains(&&*lp);
    Ok(OperandsOut {
        ok,
        op: s.op,
        l,
        r,
        operand: lp,
    })
}

pub fn op_result(op: Str, operand: Str) -> Result<Type, String> {
    let cmp = matches!(&*op, "==" | "!=" | "<" | "<=" | ">" | ">=");
    Ok(Type::Prim(if cmp { "bool".into() } else { operand }))
}

// ---- Switches ----

pub fn head_index(arms: List<Arm>, kind: Str) -> Result<i64, String> {
    Ok(arms
        .iter()
        .position(|a| match (&a.head, &*kind) {
            (Head::Pat(_), "pat") | (Head::Cond(_), "cond") => true,
            _ => false,
        })
        .map_or(-1, |i| i as i64))
}

pub fn slice(src: Str, span: Span) -> Result<Str, String> {
    Ok(src
        .get(span.at as usize..span.end as usize)
        .unwrap_or("")
        .trim()
        .into())
}

pub fn ends_in_else(arms: List<SArm>) -> Result<bool, String> {
    Ok(matches!(arms.last().map(|a| &a.test), Some(Test::Else)))
}

pub fn pats_of(arms: List<SArm>) -> Result<List<SPat>, String> {
    Ok(arms
        .iter()
        .filter_map(|a| match &a.test {
            Test::Pat(p) => Some(p.clone()),
            _ => None,
        })
        .collect())
}

pub fn unwrap_arms(r: ResSArmList) -> Result<List<SArm>, String> {
    match r {
        ResSArmList::Ok(a) => Ok(a),
        ResSArmList::Err(_) => err("unwrap_arms"),
    }
}

pub fn bind_name(k: PatKind) -> Result<Ident, String> {
    match k {
        PatKind::Bind(b) => Ok(b),
        _ => err("bind_name"),
    }
}

pub fn variant_tag(k: PatKind) -> Result<Ident, String> {
    match k {
        PatKind::Variant(v) => Ok(v.tag.clone()),
        _ => err("variant_tag"),
    }
}

pub fn variant_fix(t: Type) -> Result<Str, String> {
    Ok(if is_void(t)? {
        "()".into()
    } else {
        "(_)".into()
    })
}

// Exhaustiveness: the pattern matrix.

const STRUCT: &str = "{}";

fn lower(p: &SPat) -> P {
    match &*p.kind {
        SPatKind::Wild | SPatKind::Bind(_) => P::Wild,
        SPatKind::Variant(v) => P::Ctor(PCtor {
            c: v.tag.clone(),
            args: match &*v.payload {
                MaybePat::Some(q) => vec![lower(q)].into(),
                MaybePat::None => List::default(),
            },
        }),
        SPatKind::Struct(fs) => P::Ctor(PCtor {
            c: STRUCT.into(),
            args: fs.iter().map(|f| lower(&f.pat)).collect(),
        }),
        SPatKind::Or(alts) => P::Or(alts.iter().map(lower).collect()),
    }
}

pub fn rows_of(pats: List<SPat>) -> Result<List<List<P>>, String> {
    Ok(pats.iter().map(|p| List::from(vec![lower(p)])).collect())
}

fn expand_rows(rows: &[List<P>]) -> Vec<List<P>> {
    let mut out = vec![];
    for r in rows {
        match &r[0] {
            P::Or(alts) => {
                let split: Vec<List<P>> = alts
                    .iter()
                    .map(|a| List::from(vec![a.clone()]).concat(&r[1..].into()))
                    .collect();
                out.extend(expand_rows(&split));
            }
            _ => out.push(r.clone()),
        }
    }
    out
}

pub fn specialize(rows: List<List<P>>, c: Str, arity: i64) -> Result<List<List<P>>, String> {
    Ok(expand_rows(&rows)
        .into_iter()
        .filter_map(|r| match &r[0] {
            P::Wild => Some(List::from(vec![P::Wild; arity as usize]).concat(&r[1..].into())),
            P::Ctor(d) if d.c == c => Some(d.args.clone().concat(&r[1..].into())),
            _ => None,
        })
        .collect())
}

pub fn default_rows(rows: List<List<P>>) -> Result<List<List<P>>, String> {
    Ok(expand_rows(&rows)
        .into_iter()
        .filter(|r| matches!(r[0], P::Wild))
        .map(|r| List::from(&r[1..]))
        .collect())
}

pub fn used(rows: List<List<P>>) -> Result<List<Str>, String> {
    Ok(expand_rows(&rows)
        .iter()
        .filter_map(|r| match &r[0] {
            P::Ctor(c) => Some(c.c.clone()),
            _ => None,
        })
        .collect())
}

pub fn ctors(t: Type) -> Result<List<Ctor>, String> {
    Ok(match t {
        Type::Enum(vs) => vs
            .iter()
            .map(|m| Ctor {
                c: m.name.clone(),
                args: if matches!(&m.ty, Type::Prim(p) if &**p == "void") {
                    List::default()
                } else {
                    vec![m.ty.clone()].into()
                },
            })
            .collect(),
        Type::Struct(fs) => vec![Ctor {
            c: STRUCT.into(),
            args: fs.iter().map(|m| m.ty.clone()).collect(),
        }]
        .into(),
        _ => List::default(),
    })
}

pub fn all_used(cs: List<Ctor>, used: List<Str>) -> Result<bool, String> {
    Ok(cs.iter().all(|c| used.contains(&c.c)))
}

pub fn wilds(n: i64) -> Result<List<P>, String> {
    Ok(vec![P::Wild; n as usize].into())
}

pub fn rest_p(ps: List<P>) -> Result<List<P>, String> {
    Ok(ps[1..].into())
}

pub fn ctor_of(p: P) -> Result<PCtor, String> {
    match p {
        P::Ctor(c) => Ok(c),
        _ => err("ctor_of"),
    }
}

pub fn ctor_args(cs: List<Ctor>, c: Str) -> Result<List<Type>, String> {
    cs.iter()
        .find(|d| d.c == c)
        .map(|d| d.args.clone())
        .ok_or_else(|| "ctor_args: missing".into())
}

pub fn unnamed_ctor(cs: List<Ctor>, used: List<Str>) -> Result<W, String> {
    Ok(match cs.iter().find(|c| !used.contains(&c.c)) {
        Some(c) => W::Ctor(WCtor {
            c: c.c.clone(),
            args: vec![W::Wild; c.args.len()].into(),
        }),
        None => W::Wild,
    })
}

pub fn regroup(c: Str, n: i64, ws: List<W>) -> Result<List<W>, String> {
    let n = n as usize;
    Ok(List::from(vec![W::Ctor(WCtor {
        c,
        args: ws[..n].into(),
    })])
    .concat(&ws[n..].into()))
}

pub fn ctor_name(w: W) -> Result<Str, String> {
    match w {
        W::Ctor(c) => Ok(c.c),
        W::Wild => err("ctor_name"),
    }
}

// ---- Graphs ----

fn graph_of(mut g: Graph, f: impl FnOnce(&mut Graph)) -> Graph {
    f(&mut g);
    g
}

pub fn new_region(g: Graph, parent: i64, entry: i64, elem: Type) -> Result<Graph, String> {
    Ok(graph_of(g, |g| {
        g.regions = push(
            g.regions.clone(),
            Region {
                parent,
                entry,
                elem,
                nodes: List::default(),
                sink: -1,
            },
        )
    }))
}

pub fn in_region(g: Graph, n: i64, r: i64) -> Result<Graph, String> {
    Ok(graph_of(g, |g| {
        g.region_of = g.region_of.clone().merge(&Map::from_iter([(n, r)]));
        g.regions = updated(g.regions.clone(), r, |reg| {
            reg.nodes = push(reg.nodes.clone(), n)
        });
    }))
}

pub fn with_sink(g: Graph, r: i64, n: i64) -> Result<Graph, String> {
    Ok(graph_of(g, |g| {
        g.regions = updated(g.regions.clone(), r, |reg| reg.sink = n)
    }))
}

pub fn with_open(g: Graph, n: i64, r: i64) -> Result<Graph, String> {
    Ok(graph_of(g, |g| {
        g.opens = g.opens.clone().merge(&Map::from_iter([(n, r)]))
    }))
}

fn arm_leaves(a: &SArm, out: &mut Vec<Call>) {
    match &a.body {
        SBody::Call(c) => out.push(c.clone()),
        SBody::Switch(arms) => arms.iter().for_each(|a| arm_leaves(a, out)),
    }
}

fn conds(a: &SArm, out: &mut Vec<Expr>) {
    if let Test::Cond(e) = &a.test {
        out.push(e.clone());
    }
    if let SBody::Switch(arms) = &a.body {
        arms.iter().for_each(|a| conds(a, out));
    }
}

fn calls_of(e: &SEdge) -> Vec<Call> {
    match e {
        SEdge::Call(c) | SEdge::Collect(c) => vec![c.clone()],
        SEdge::ForEach(f) => vec![f.call.clone()],
        SEdge::Match(arms) => {
            let mut out = vec![];
            arms.iter().for_each(|a| arm_leaves(a, &mut out));
            out
        }
        SEdge::Spawn(s) => s
            .branches
            .iter()
            .map(|b| b.call.clone())
            .chain([s.join.clone()])
            .collect(),
    }
}

pub fn edge_calls(e: SEdge) -> Result<List<Call>, String> {
    Ok(calls_of(&e).into())
}

pub fn leaf_targets(arms: List<SArm>) -> Result<List<i64>, String> {
    let mut out = vec![];
    arms.iter().for_each(|a| arm_leaves(a, &mut out));
    Ok(out.iter().map(|c| c.target).collect())
}

pub fn arg_values(calls: List<Call>) -> Result<List<Expr>, String> {
    Ok(calls
        .iter()
        .flat_map(|c| c.args.iter().map(|a| a.value.clone()).collect::<Vec<_>>())
        .collect())
}

pub fn arm_conds(arms: List<SArm>) -> Result<List<Expr>, String> {
    let mut out = vec![];
    arms.iter().for_each(|a| conds(a, &mut out));
    Ok(out.into())
}

pub fn arm_reads(arms: List<SArm>) -> Result<List<Expr>, String> {
    let mut out = vec![];
    for a in arms.iter() {
        conds(a, &mut out);
        let mut calls = vec![];
        arm_leaves(a, &mut calls);
        out.extend(
            calls
                .iter()
                .flat_map(|c| c.args.iter().map(|x| x.value.clone()).collect::<Vec<_>>()),
        );
    }
    Ok(out.into())
}

pub fn edge_nodes(g: Graph) -> Result<List<i64>, String> {
    Ok(g.edges.keys().copied().collect())
}

pub fn ancestors(g: Graph, r: i64) -> Result<Map<i64, bool>, String> {
    let mut out = vec![(r, true)];
    let mut r = r;
    while g.regions[r as usize].parent >= 0 {
        r = g.regions[r as usize].parent;
        out.push((r, true));
    }
    Ok(out.into_iter().collect())
}

pub fn opener(g: Graph, r: i64) -> Result<i64, String> {
    Ok(g.opens
        .iter()
        .find(|(_, x)| **x == r)
        .map_or(-1, |(n, _)| *n))
}

pub fn proc_entry(nodes: List<NodeDecl>, pid: i64) -> Result<i64, String> {
    match &nodes[pid as usize].kind {
        NodeKind::Proc(p) => Ok(p.entry),
        _ => err("proc_entry"),
    }
}

pub fn unwrap_g(r: ResGraph) -> Result<Graph, String> {
    match r {
        ResGraph::Ok(g) => Ok(g),
        ResGraph::Err(_) => err("unwrap_g"),
    }
}

pub fn for_each_target(k: EdgeKind) -> Result<Target, String> {
    match k {
        EdgeKind::ForEach(f) => Ok(f.target),
        _ => err("for_each_target"),
    }
}

pub fn for_each_dim(k: EdgeKind) -> Result<Value, String> {
    match k {
        EdgeKind::ForEach(f) => Ok(f.dim),
        _ => err("for_each_dim"),
    }
}

pub fn for_each_call(e: SEdge) -> Result<Call, String> {
    match e {
        SEdge::ForEach(f) => Ok(f.call.clone()),
        _ => err("for_each_call"),
    }
}

pub fn collect_target(g: Graph, sink: i64) -> Result<i64, String> {
    match g.edges.get(&sink) {
        Some(SEdge::Collect(c)) => Ok(c.target),
        _ => err("collect_target"),
    }
}

pub fn root_node(r: Root) -> Result<i64, String> {
    Ok(match r {
        Root::Node(n) => n,
        _ => -1,
    })
}

pub fn expr_parts(e: Expr) -> Result<List<Expr>, String> {
    let one = |x: &Expr| vec![x.clone()];
    Ok(match &e {
        Expr::Ref(_) | Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) => vec![],
        Expr::Variant(v) => match &*v.payload {
            MaybeExpr::Some(p) => one(p),
            MaybeExpr::None => vec![],
        },
        Expr::Struct(fs) => fs.iter().map(|f| f.e.clone()).collect(),
        Expr::Array(es) => es.to_vec(),
        Expr::Unary(u) => one(&u.e),
        Expr::Binary(b) => vec![b.l.as_ref().clone(), b.r.as_ref().clone()],
        Expr::Call(c) => c.args.iter().map(|a| a.value.clone()).collect(),
        Expr::Map(kvs) => kvs
            .iter()
            .flat_map(|kv| [kv.key.clone(), kv.value.clone()])
            .collect(),
        Expr::Field(f) => one(&f.e),
        Expr::Query(q) => one(&q.e),
        Expr::Index(x) => vec![x.e.as_ref().clone(), x.key.as_ref().clone()],
        Expr::In(x) => vec![x.key.as_ref().clone(), x.map.as_ref().clone()],
        Expr::Merge(m) => vec![m.l.as_ref().clone(), m.r.as_ref().clone()],
        Expr::Remove(m) => vec![m.m.as_ref().clone(), m.keys.as_ref().clone()],
    }
    .into())
}

pub fn expr_ref(e: Expr) -> Result<List<Ref>, String> {
    Ok(match e {
        Expr::Ref(r) => vec![r].into(),
        _ => List::default(),
    })
}

pub fn rest_exprs(es: List<Expr>) -> Result<List<Expr>, String> {
    Ok(es[1..].into())
}

fn successors(g: &Graph, n: i64) -> Vec<i64> {
    match g.edges.get(&n) {
        Some(SEdge::Call(c)) => vec![c.target],
        Some(SEdge::Match(arms)) => {
            let mut out = vec![];
            arms.iter().for_each(|a| arm_leaves(a, &mut out));
            out.iter().map(|c| c.target).collect()
        }
        Some(SEdge::ForEach(_)) => {
            let sink = g.regions[g.opens[&n] as usize].sink;
            match g.edges.get(&sink) {
                Some(SEdge::Collect(c)) => vec![c.target],
                _ => vec![],
            }
        }
        // The branches run inside the spawner's step.
        Some(SEdge::Spawn(s)) => vec![s.join.target],
        Some(SEdge::Collect(_)) | None => vec![],
    }
}

// ---- Spawns ----

fn diag(msg: String, span: Span) -> Diag {
    Diag {
        msg: msg.into(),
        span,
    }
}

fn qual(nodes: &List<NodeDecl>, n: i64) -> String {
    let q: Vec<&str> = nodes[n as usize].qual.iter().map(|s| &**s).collect();
    q.join(".")
}

/// A spawn's targets as written: the branches', then the join's.
pub fn spawn_targets(branches: List<Branch>, join: Target) -> Result<List<Target>, String> {
    Ok(branches
        .iter()
        .map(|b| b.target.clone())
        .chain([join])
        .collect())
}

pub fn has_id(t: Target) -> Result<bool, String> {
    Ok(matches!(t.id, OptValue::Some(_)))
}

pub fn has_branch_target(out: List<SBranch>, n: i64) -> Result<bool, String> {
    Ok(out.iter().any(|b| b.call.target == n))
}

pub fn last_name(n: NodeDecl) -> Result<Str, String> {
    Ok(n.qual.last().cloned().unwrap_or_default())
}

/// The first `this.X` argument of the join naming a detached branch X.
pub fn detached_ref(
    join: Target,
    out: List<SBranch>,
    nodes: List<NodeDecl>,
) -> Result<OptIdent, String> {
    for a in join.args.iter() {
        let ValueKind::Ref(path) = &*a.value.kind else {
            continue;
        };
        let (Some(this), Some(x)) = (path.segs.first(), path.segs.get(1)) else {
            continue;
        };
        let detached = |b: &SBranch| {
            b.detach
                && nodes[b.call.target as usize]
                    .qual
                    .last()
                    .is_some_and(|l| *l == x.name)
        };
        if &*this.name == "this" && out.iter().any(detached) {
            return Ok(OptIdent::Some(x.clone()));
        }
    }
    Ok(OptIdent::None)
}

/// Records each spawn's branches: each runs in one branch only, with no
/// edge of its own. Spawners in node order.
pub fn with_spawned(g: Graph, nodes: List<NodeDecl>) -> Result<ResGraph, String> {
    let mut spawned: std::collections::BTreeMap<i64, i64> = Default::default();
    for (&src, e) in g.edges.iter() {
        let SEdge::Spawn(s) = e else { continue };
        for b in s.branches.iter() {
            let n = b.call.target;
            if spawned.insert(n, src).is_some() {
                let msg = format!(
                    "{} runs in more than one spawn branch; name each with `as`",
                    qual(&nodes, n)
                );
                return Ok(ResGraph::Err(diag(msg, b.call.span.clone())));
            }
            if g.edges.contains_key(&n) {
                let msg = format!(
                    "{} runs in a spawn, so it can't have an edge of its own: its output goes to the join",
                    qual(&nodes, n)
                );
                return Ok(ResGraph::Err(diag(msg, b.call.span.clone())));
            }
        }
    }
    Ok(ResGraph::Ok(Graph {
        spawned: spawned.into_iter().collect(),
        ..g
    }))
}

/// Fails if a branch is also reached as a step of the proc.
pub fn spawn_reach(g: Graph, nodes: List<NodeDecl>, span: Span) -> Result<ResGraph, String> {
    match g.spawned.keys().find(|n| g.region_of.contains_key(n)) {
        Some(&n) => {
            let msg = format!(
                "{} runs in a spawn and is also called directly; name one with `as`",
                qual(&nodes, n)
            );
            Ok(ResGraph::Err(diag(msg, span)))
        }
        None => Ok(ResGraph::Ok(g)),
    }
}

pub fn preds(g: Graph, r: i64) -> Result<Map<i64, List<i64>>, String> {
    let mut out: std::collections::BTreeMap<i64, Vec<i64>> = Default::default();
    for &n in g.regions[r as usize].nodes.iter() {
        for t in successors(&g, n) {
            if g.region_of.get(&t) == Some(&r) {
                out.entry(t).or_default().push(n);
            }
        }
    }
    Ok(out.into_iter().map(|(k, v)| (k, List::from(v))).collect())
}

pub fn dom_start(
    g: Graph,
    r: i64,
    dom: Map<i64, Map<i64, bool>>,
) -> Result<Map<i64, Map<i64, bool>>, String> {
    let region = &g.regions[r as usize];
    let all: Map<i64, bool> = region.nodes.iter().map(|&n| (n, true)).collect();
    let fresh: Map<i64, Map<i64, bool>> = region
        .nodes
        .iter()
        .map(|&n| {
            (
                n,
                if n == region.entry {
                    Map::from_iter([(n, true)])
                } else {
                    all.clone()
                },
            )
        })
        .collect();
    Ok(dom.merge(&fresh))
}

pub fn meet(
    preds: Map<i64, List<i64>>,
    dom: Map<i64, Map<i64, bool>>,
    n: i64,
) -> Result<Map<i64, bool>, String> {
    let mut acc: Option<std::collections::BTreeSet<i64>> = None;
    for p in preds.get(&n).map(|ps| ps.to_vec()).unwrap_or_default() {
        let d: std::collections::BTreeSet<i64> = dom[&p].keys().copied().collect();
        acc = Some(match acc {
            None => d,
            Some(a) => a.intersection(&d).copied().collect(),
        });
    }
    let mut d = acc.unwrap_or_default();
    d.insert(n);
    Ok(d.into_iter().map(|x| (x, true)).collect())
}

pub fn same_set(a: Map<i64, bool>, b: Map<i64, bool>) -> Result<bool, String> {
    Ok(a.keys().eq(b.keys()))
}

pub fn skip(
    g: Graph,
    dom: Map<i64, Map<i64, bool>>,
    n: i64,
    r: i64,
    at: i64,
) -> Result<i64, String> {
    let (mut r, mut at) = (r, at);
    loop {
        if g.region_of.get(&n) == Some(&r) {
            return Ok(if dom[&at].contains_key(&n) { -1 } else { at });
        }
        match opener(g.clone(), r)? {
            -1 => return Ok(at),
            o => {
                at = o;
                r = g.region_of[&o];
            }
        }
    }
}

pub fn set_graph(mut cx: Cx, pid: i64, g: Graph) -> Result<Cx, String> {
    cx.nodes = updated(cx.nodes, pid, |n| {
        if let NodeKind::Proc(p) = &mut n.kind {
            p.graph = g;
        }
    });
    Ok(cx)
}

pub fn pack_placed(l: Layout, _target: i64, back: bool) -> Result<RecordPlacedOut, String> {
    Ok(RecordPlacedOut { l, back })
}

pub fn extra_member(bfs: List<Member>, fs: List<Member>) -> Result<Str, String> {
    Ok(bfs
        .iter()
        .find(|b| !fs.iter().any(|f| f.name == b.name))
        .map_or("".into(), |b| b.name.clone()))
}

pub fn squash(src: Str, span: Span) -> Result<Str, String> {
    let text = src.get(span.at as usize..span.end as usize).unwrap_or("");
    Ok(text.split_whitespace().collect::<Vec<_>>().join(" ").into())
}

// ---- id(..) as N ----

pub fn untyped(n: NodeDecl, span: Span) -> Result<Diag, String> {
    let name = n.qual.last().cloned().unwrap_or_default();
    Ok(diag(
        format!(
            "{name} is id(..) as {name}, whose type is that of the values passed to it, and none of them has a type yet; pass it a value whose type is known first"
        ),
        span,
    ))
}

/// Whether proc `pid` has an `id(..) as N` with no type yet.
pub fn has_untyped(nodes: List<NodeDecl>, pid: i64) -> Result<bool, String> {
    let NodeKind::Proc(p) = &nodes[pid as usize].kind else {
        return err("has_untyped: not a proc");
    };
    Ok(p.locals
        .iter()
        .any(|&l| is_untyped_id(nodes[l as usize].clone()).unwrap_or(false)))
}

pub fn types_ids(nodes: List<NodeDecl>, e: SEdge) -> Result<bool, String> {
    Ok(calls_of(&e)
        .iter()
        .any(|c| is_untyped_id(nodes[c.target as usize].clone()).unwrap_or(false)))
}

/// nodes with each untyped id the edge calls typed by its first call's value.
pub fn typed_ids(mut nodes: List<NodeDecl>, e: SEdge) -> Result<List<NodeDecl>, String> {
    for c in calls_of(&e).iter() {
        if is_untyped_id(nodes[c.target as usize].clone())? {
            let t = c.args.first().ok_or("typed_ids: no value")?.param.clone();
            nodes = with_id_type(nodes, c.target, t)?;
        }
    }
    Ok(nodes)
}
