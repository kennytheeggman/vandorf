//! Leaves for vcc's Go backend (go.sys): names, types and the text around
//! the procs' state machines, after src/codegen/go.rs.

use super::gen_leaf::{
    RAW_REQUEST, RAW_RESPONSE, base_of, find_top, get, gname, join_path, live_after, member_of,
    quote, raw_shape, read_nodes, refs_of, render_lines, rs_impl, show_shape, show_type, ty_boxed,
};
use crate::vcc::types::*;

fn err<T>(what: &str) -> Result<T, String> {
    Err(format!("go leaf: {what}"))
}

fn exported(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| {
        f.to_ascii_uppercase().to_string() + c.as_str()
    })
}

fn q(s: &str) -> String {
    quote(s.into()).unwrap().to_string()
}

fn gn(nodes: &List<NodeDecl>, n: i64) -> String {
    gname(nodes[n as usize].clone()).unwrap().to_string()
}

fn qn(nodes: &List<NodeDecl>, n: i64) -> String {
    super::sema_leaf::qual_name(nodes[n as usize].clone())
        .unwrap()
        .to_string()
}

fn params_of(nodes: &List<NodeDecl>, n: i64) -> List<Member> {
    super::sema_leaf::node_params(nodes.clone(), n).unwrap()
}

fn ret_of(nodes: &List<NodeDecl>, n: i64) -> Type {
    super::sema_leaf::node_ret(nodes.clone(), n).unwrap()
}

fn graph(nodes: &List<NodeDecl>, pid: i64) -> Graph {
    super::doc_leaf::graph_of(nodes[pid as usize].clone()).unwrap()
}

fn is_void(t: &Type) -> bool {
    super::sema_leaf::is_void(t.clone()).unwrap()
}

fn unfold<'a>(mut t: &'a Type, defs: &'a [TypeDef]) -> &'a Type {
    while let Type::Named(n) = t {
        t = &defs[n.id as usize].ty;
    }
    t
}

fn boxed(t: &Table, ty: &Type, part: &str) -> bool {
    ty_boxed(t.clone(), ty.clone(), part.into()).unwrap()
}

fn lines(ls: Vec<String>) -> List<Str> {
    ls.into_iter().map(Str::from).collect()
}

fn render(ls: Vec<String>) -> String {
    render_lines(
        ls.into_iter().map(|l| Line::L(l.into())).collect(),
        "\t".into(),
    )
    .unwrap()
    .to_string()
}

// ---- Names ----

/// A Go identifier from a path segment.
fn ident(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("p{s}")
    } else {
        s
    }
}

const GO_RESERVED: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
    "bool",
    "int",
    "float64",
    "string",
    "error",
    "any",
    "ctx",
    "env",
    "pc",
    "zero",
    "this",
    "err",
    "elem",
    "results",
];

fn go_param(s: &str) -> String {
    if GO_RESERVED.contains(&s) {
        format!("{s}_")
    } else {
        s.to_string()
    }
}

fn check_fields<'a>(names: impl Iterator<Item = &'a str>, is_enum: bool) -> Result<(), String> {
    let mut seen: std::collections::HashMap<String, &str> = Default::default();
    if is_enum {
        seen.insert("Tag".into(), "the enum tag");
    }
    for n in names {
        if let Some(prev) = seen.insert(exported(n), n) {
            return Err(format!(
                "{prev} and {n} collide as Go field {}",
                exported(n)
            ));
        }
    }
    Ok(())
}

pub fn go_name_check(prog: Program, t: Table) -> Result<MaybeStr, String> {
    let fail = |e: String| Ok(MaybeStr::Some(e.into()));
    let mut seen: std::collections::HashMap<String, String> = Default::default();
    for (id, n) in prog.nodes.iter().enumerate() {
        let g = gn(&prog.nodes, id as i64);
        let qual = qn(&prog.nodes, id as i64);
        if let Some(prev) = seen.insert(g.clone(), qual.clone()) {
            return fail(format!("{prev} and {qual} both map to the Go name {g}"));
        }
        if let NodeKind::Func(f) = &n.kind
            && let Err(e) = check_fields(f.params.iter().map(|m| &*m.name), false)
        {
            return fail(format!("{qual}: {e}"));
        }
    }
    for n in t.types.iter() {
        let (Shape::Struct(ms) | Shape::Enum(ms)) = &n.shape;
        if let Err(e) = check_fields(
            ms.iter().map(|m| &*m.name),
            matches!(n.shape, Shape::Enum(_)),
        ) {
            return fail(format!("{} ({}): {e}", n.name, n.origin));
        }
    }
    Ok(MaybeStr::None)
}

pub fn go_setup(package: Str, import_base: MaybeStr, types_dir: Str) -> Result<GoSetup, String> {
    let dir = types_dir.trim_matches('/');
    let fail = |e: String| GoSetup {
        pkg: "".into(),
        import: MaybeStr::None,
        dir: "".into(),
        tq: "".into(),
        err: MaybeStr::Some(e.into()),
    };
    if matches!(dir, "" | ".") {
        return Ok(GoSetup {
            pkg: package,
            import: MaybeStr::None,
            dir: "".into(),
            tq: "".into(),
            err: MaybeStr::None,
        });
    }
    let MaybeStr::Some(base) = import_base else {
        return Ok(fail(
            "the types package needs an import path: pass --module, run inside a Go module, or put types in the code package (--types-dir .)".into(),
        ));
    };
    let MaybeStr::Some(import) = join_path(base, dir.into())? else {
        return Ok(fail(format!(
            "cannot resolve import path for types dir {dir}"
        )));
    };
    let pkg = ident(import.rsplit('/').next().unwrap());
    Ok(GoSetup {
        tq: format!("{pkg}.").into(),
        pkg: pkg.into(),
        import: MaybeStr::Some(import),
        dir: dir.into(),
        err: MaybeStr::None,
    })
}

pub fn go_in_type(nodes: List<NodeDecl>, n: i64) -> Result<Str, String> {
    match &nodes[n as usize].kind {
        NodeKind::Func(_) | NodeKind::Id(_) | NodeKind::Sig(_) => {
            Ok(format!("in{}", gn(&nodes, n)).into())
        }
        NodeKind::Proc(p) => go_in_type(nodes.clone(), p.entry),
        NodeKind::Alias(b) => go_in_type(nodes.clone(), *b),
    }
}

fn in_type(nodes: &List<NodeDecl>, n: i64) -> String {
    go_in_type(nodes.clone(), n).unwrap().to_string()
}

fn pc(nodes: &List<NodeDecl>, pid: i64, n: i64) -> String {
    format!("pc{}_{}", gn(nodes, pid), gn(nodes, n))
}

pub fn go_pc(nodes: List<NodeDecl>, pid: i64, n: i64) -> Result<Str, String> {
    Ok(pc(&nodes, pid, n).into())
}

fn region_fn(nodes: &List<NodeDecl>, pid: i64, r: i64) -> String {
    format!("run{}Region{r}", gn(nodes, pid))
}

pub fn go_proc_file(n: NodeDecl) -> Result<Str, String> {
    Ok(format!(
        "{}_vandorf.go",
        n.qual.last().map_or("", |s| &**s).to_lowercase()
    )
    .into())
}

// ---- Types ----

pub fn go_render(t: Table, r: TyRef, qual: Str) -> Result<Str, String> {
    Ok(match r {
        TyRef::Prim(p) => match &*p {
            "void" => "struct{}",
            "int" => "int",
            "float" => "float64",
            "bool" => "bool",
            _ => "string",
        }
        .into(),
        TyRef::Array(e) => format!("[]{}", go_render(t, (*e).clone(), qual)?).into(),
        TyRef::Map(m) => format!(
            "map[{}]{}",
            go_render(t.clone(), (*m.key).clone(), qual.clone())?,
            go_render(t, (*m.value).clone(), qual)?
        )
        .into(),
        TyRef::Named(i) => format!("{qual}{}", t.types[i as usize].name).into(),
    })
}

fn gtype(t: &Table, tq: &str, ty: &Type) -> String {
    go_render(t.clone(), get(t.clone(), ty.clone()).unwrap(), tq.into())
        .unwrap()
        .to_string()
}

/// A vandorf type as written in the code package.
pub fn go_type(g: GGen, ty: Type) -> Result<Str, String> {
    Ok(gtype(&g.table, &g.tq, &ty).into())
}

pub fn go_doc(n: NamedType) -> Result<Line, String> {
    Ok(Line::L(
        if *n.origin == *format!("typedef {}", n.name) {
            format!("// {} is a typedef.", n.name)
        } else {
            format!("// {} is the type of the {}.", n.name, n.origin)
        }
        .into(),
    ))
}

pub fn go_tag_doc(ms: List<RefMember>) -> Result<Str, String> {
    let tags: Vec<String> = ms.iter().map(|m| q(&m.name)).collect();
    Ok(format!(
        "// Tag is one of {}; the field it names holds the payload.",
        tags.join(", ")
    )
    .into())
}

pub fn go_field_type(t: Table, i: i64, m: RefMember) -> Result<Str, String> {
    let s = go_render(t.clone(), m.r, "".into())?;
    Ok(
        if t.boxed.contains_key(&Str::from(format!("{i}:{}", m.name))) {
            format!("*{s}").into()
        } else {
            s
        },
    )
}

// ---- The generator's state ----

pub fn gw(mut g: GGen, ls: List<Str>) -> Result<GGen, String> {
    let more: Vec<Line> = ls.iter().map(|l| Line::L(l.clone())).collect();
    g.w = g.w.concat(&more.into());
    Ok(g)
}

pub fn go_failed(mut g: GGen, msg: Str) -> Result<GGen, String> {
    if let MaybeStr::None = g.failed {
        g.failed = MaybeStr::Some(msg);
    }
    Ok(g)
}

pub fn go_fatal(mut g: GGen, msg: Str) -> Result<GGen, String> {
    if let MaybeStr::None = g.fatal {
        g.fatal = MaybeStr::Some(msg);
    }
    Ok(g)
}

pub fn go_coerce_key(at: GCoerceAt) -> Result<Str, String> {
    Ok(format!(
        "{} -> {}",
        gtype(&at.g.table, &at.g.tq, &at.from),
        gtype(&at.g.table, &at.g.tq, &at.to)
    )
    .into())
}

pub fn go_coercion_name(at: GCoerceAt) -> Result<Str, String> {
    Ok(format!("coerce{}", at.g.coercions.len()).into())
}

pub fn go_new_coercion(at: GCoerceAt) -> Result<GGen, String> {
    let (key, name) = (go_coerce_key(at.clone())?, go_coercion_name(at.clone())?);
    let mut g = at.g;
    g.coercions = g.coercions.merge(&Map::from_iter([(key, name)]));
    Ok(g)
}

pub fn go_coerce_head(at: GCoerceAt) -> Result<Str, String> {
    Ok(format!(
        "func {}(v {}) (r {}) {{",
        go_coercion_name(at.clone())?,
        gtype(&at.g.table, &at.g.tq, &at.from),
        gtype(&at.g.table, &at.g.tq, &at.to)
    )
    .into())
}

pub fn go_lines(ls: List<Str>) -> Result<Str, String> {
    Ok(render(ls.iter().map(|l| l.to_string()).collect()).into())
}

// ---- Expressions ----

pub fn go_ref(g: GGen, r: Ref) -> Result<Str, String> {
    let mut s = match r.root {
        Root::This => "this".to_string(),
        Root::Node(n) => format!("env.out{}", gn(&g.prog.nodes, n)),
        Root::Bind => format!("b_{}", r.bind),
    };
    // Go selects through pointers implicitly; only a boxed last field needs
    // dereferencing.
    let mut ty = r.root_ty.clone();
    let mut is_boxed = false;
    for f in r.fields.iter() {
        is_boxed = boxed(&g.table, &ty, f);
        ty = member_of(g.prog.types.clone(), ty, f.clone())?;
        s.push('.');
        s.push_str(&exported(f));
    }
    Ok(if is_boxed { format!("(*{s})") } else { s }.into())
}

pub fn go_float(f: f64) -> Result<Str, String> {
    Ok(format!("{f:?}").into())
}

pub fn go_box_if(b: bool, s: Str) -> Result<Str, String> {
    Ok(if b { format!("ptrTo({s})").into() } else { s })
}

fn fails(e: &Expr) -> bool {
    match e {
        Expr::Call(_) | Expr::Index(_) => true,
        Expr::Ref(_) | Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) => false,
        Expr::Variant(v) => matches!(&*v.payload, MaybeExpr::Some(p) if fails(p)),
        Expr::Struct(fs) => fs.iter().any(|m| fails(&m.e)),
        Expr::Array(es) => es.iter().any(fails),
        Expr::Map(kvs) => kvs.iter().any(|kv| fails(&kv.key) || fails(&kv.value)),
        Expr::Unary(x) => fails(&x.e),
        Expr::Field(x) => fails(&x.e),
        Expr::Query(x) => fails(&x.e),
        Expr::Binary(x) => fails(&x.l) || fails(&x.r),
        Expr::In(x) => fails(&x.key) || fails(&x.map),
        Expr::Merge(x) => fails(&x.l) || fails(&x.r),
        Expr::Remove(x) => fails(&x.m) || fails(&x.keys),
    }
}

pub fn can_fail(e: Expr) -> Result<bool, String> {
    Ok(fails(&e))
}

pub fn in_map_e(e: Expr) -> Result<Expr, String> {
    match e {
        Expr::In(x) => Ok((*x.map).clone()),
        _ => err("in_map_e"),
    }
}

pub fn go_call_lines(t: Str, call: Str, qual: Str) -> Result<List<Str>, String> {
    Ok(lines(vec![
        format!("{t}, err := {call}"),
        "if err != nil {".into(),
        format!(
            "return zero, fmt.Errorf({}, err)",
            q(&format!("{qual}: %w"))
        ),
        "}".into(),
    ]))
}

pub fn go_short_if(op: Str, t: Str) -> Result<Str, String> {
    Ok(if &*op == "&&" {
        format!("if {t} {{")
    } else {
        format!("if !{t} {{")
    }
    .into())
}

pub fn go_index_lines(t: Str, x: Str, k: Str, map: bool, proc: Str) -> Result<List<Str>, String> {
    let get = if map { "mapGet" } else { "sliceGet" };
    Ok(lines(vec![
        format!("{t}, err := {get}({x}, {k})"),
        "if err != nil {".into(),
        format!(
            "return zero, fmt.Errorf({}, err)",
            q(&format!("{proc}: %w"))
        ),
        "}".into(),
    ]))
}

pub fn go_select(x: Str, name: Str, boxed: bool) -> Result<Str, String> {
    let v = format!("({x}).{}", exported(&name));
    Ok(if boxed { format!("(*{v})") } else { v }.into())
}

pub fn go_callee(g: GGen, n: i64) -> Result<GoCallee, String> {
    let nodes = &g.prog.nodes;
    let n = base_of(nodes.clone(), n)?;
    let out = |s: String, stub: i64, path: String, alias: String| GoCallee {
        s: s.into(),
        stub,
        path: path.into(),
        alias: alias.into(),
        err: MaybeStr::None,
    };
    let fail = |e: String| GoCallee {
        s: "".into(),
        stub: -1,
        path: "".into(),
        alias: "".into(),
        err: MaybeStr::Some(e.into()),
    };
    let MaybeImpl::Some(imp) = rs_impl(nodes[n as usize].clone(), "go".into())? else {
        return Ok(out(
            format!("unimplemented{}", gn(nodes, n)),
            n,
            "".into(),
            "".into(),
        ));
    };
    let dir = imp
        .file
        .rsplit_once('/')
        .map_or("", |(d, _)| d)
        .trim_end_matches('/');
    if dir.is_empty() || dir == "." {
        return Ok(out(imp.function.to_string(), -1, "".into(), "".into()));
    }
    let qual = qn(nodes, n);
    let MaybeStr::Some(base) = &g.import_base else {
        return Ok(fail(format!(
            "{qual} is implemented in {}, outside the code package, but its import path is unknown (pass --module or run inside a Go module)",
            imp.file
        )));
    };
    let path = match join_path(base.clone(), dir.into())? {
        MaybeStr::Some(p) if !p.is_empty() => p.to_string(),
        _ => {
            return Ok(fail(format!(
                "{qual}: {} is outside the Go module {base} is in",
                imp.file
            )));
        }
    };
    if let Some(a) = g.imports.get(&Str::from(path.as_str())) {
        return Ok(out(
            format!("{a}.{}", imp.function),
            -1,
            "".into(),
            "".into(),
        ));
    }
    let stem = ident(path.rsplit('/').next().unwrap());
    let taken = |a: &str| {
        g.imports.values().any(|x| &**x == a)
            || ["context", "fmt"].contains(&a)
            || format!("{a}.") == *g.tq
    };
    let (mut alias, mut i) = (stem.clone(), 2);
    while taken(&alias) {
        alias = format!("{stem}{i}");
        i += 1;
    }
    Ok(out(format!("{alias}.{}", imp.function), -1, path, alias))
}

// ---- Patterns ----

/// The tests matching p against the Go value x (a tag checked before its
/// payload is read) and the bindings' values.
fn pat(t: &Table, p: &SPat, x: &str, tests: &mut Vec<String>, binds: &mut Vec<(String, String)>) {
    let part = |name: &str| {
        let v = format!("{x}.{}", exported(name));
        if boxed(t, &p.ty, name) {
            format!("(*{v})")
        } else {
            v
        }
    };
    match &*p.kind {
        SPatKind::Wild => {}
        SPatKind::Bind(b) => binds.push((b.to_string(), x.to_string())),
        SPatKind::Variant(v) => {
            tests.push(format!("{x}.Tag == {}", q(&v.tag)));
            if let MaybePat::Some(sub) = &*v.payload {
                pat(t, sub, &part(&v.tag), tests, binds);
            }
        }
        SPatKind::Struct(fs) => {
            for f in fs.iter() {
                pat(t, &f.pat, &part(&f.name), tests, binds);
            }
        }
        SPatKind::Or(alts) => {
            let alts: Vec<String> = alts
                .iter()
                .map(|a| {
                    let mut ts = vec![];
                    pat(t, a, x, &mut ts, &mut vec![]);
                    if ts.is_empty() {
                        "true".to_string()
                    } else {
                        format!("({})", ts.join(" && "))
                    }
                })
                .collect();
            tests.push(format!("({})", alts.join(" || ")));
        }
    }
}

pub fn go_pat_head(t: Table, p: SPat) -> Result<List<Str>, String> {
    let (mut tests, mut binds) = (vec![], vec![]);
    pat(&t, &p, "this", &mut tests, &mut binds);
    let mut out = vec![if tests.is_empty() {
        "{".to_string()
    } else {
        format!("if {} {{", tests.join(" && "))
    }];
    for (b, v) in binds {
        out.push(format!("b_{b} := {v}"));
        out.push(format!("_ = b_{b}"));
    }
    Ok(lines(out))
}

pub fn go_pat_always(t: Table, p: SPat) -> Result<bool, String> {
    let mut tests = vec![];
    pat(&t, &p, "this", &mut tests, &mut vec![]);
    Ok(tests.is_empty())
}

// ---- Procs ----

pub fn go_prelude(g: GGen, pid: i64) -> Result<List<Str>, String> {
    let nodes = &g.prog.nodes;
    let NodeKind::Proc(p) = &nodes[pid as usize].kind else {
        return err("go_prelude");
    };
    let gr = graph(nodes, pid);
    let name = gn(nodes, pid);
    let env = format!("env{name}");
    let order: Vec<i64> = gr.regions.iter().flat_map(|r| r.nodes.to_vec()).collect();
    let mut out = vec![format!("type {env} struct {{")];
    for &n in &order {
        let id = gn(nodes, n);
        out.push(format!("in{id} {}", in_type(nodes, n)));
        out.push(format!(
            "out{id} {}",
            gtype(&g.table, &g.tq, &ret_of(nodes, n))
        ));
    }
    out.extend(["}".into(), "".into(), "const (".into()]);
    for (i, &n) in order.iter().enumerate() {
        out.push(if i == 0 {
            format!("{} = iota", pc(nodes, pid, n))
        } else {
            pc(nodes, pid, n)
        });
    }
    out.extend([")".into(), "".into()]);
    let params = params_of(nodes, pid);
    let ret = gtype(&g.table, &g.tq, &p.ret);
    let sig: String = params
        .iter()
        .map(|m| format!(", {} {}", go_param(&m.name), gtype(&g.table, &g.tq, &m.ty)))
        .collect();
    out.push(format!("// Run{name} runs proc {}.", qn(nodes, pid)));
    out.push(format!(
        "func Run{name}(ctx context.Context{sig}) ({ret}, error) {{"
    ));
    out.push(format!("var zero {ret}"));
    out.push(format!("env := &{env}{{}}"));
    let fields: Vec<String> = params
        .iter()
        .map(|m| format!("{}: {}", exported(&m.name), go_param(&m.name)))
        .collect();
    out.push(format!(
        "env.in{} = {}{{{}}}",
        gn(nodes, p.entry),
        in_type(nodes, p.entry),
        fields.join(", ")
    ));
    out.push(format!("pc := {}", pc(nodes, pid, p.entry)));
    Ok(lines(out))
}

pub fn go_node_head(g: GGen, pid: i64, n: i64, call: Str) -> Result<List<Str>, String> {
    let nodes = &g.prog.nodes;
    let id = gn(nodes, n);
    let base = base_of(nodes.clone(), n)?;
    let mut call = format!("{call}(ctx");
    for m in params_of(nodes, n).iter() {
        call.push_str(&format!(", env.in{id}.{}", exported(&m.name)));
    }
    call.push(')');
    let mut out = vec![format!("case {}:", pc(nodes, pid, n))];
    let is_id = matches!(nodes[n as usize].kind, NodeKind::Id(_) | NodeKind::Sig(_));
    if let NodeKind::Sig(_) = &nodes[n as usize].kind {
        // A signature node: its output is its input's fields.
        let ty = ret_of(nodes, n);
        let fields = params_of(nodes, n)
            .iter()
            .map(|m| {
                let f = exported(&m.name);
                let v = format!("env.in{id}.{f}");
                if super::gen_leaf::ty_boxed(g.table.clone(), ty.clone(), m.name.clone())
                    .unwrap_or(false)
                {
                    format!("{f}: ptrTo({v})")
                } else {
                    format!("{f}: {v}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push(format!(
            "this := {}{{{fields}}}",
            gtype(&g.table, &g.tq, &ty)
        ));
    } else if is_id {
        // id: the value is the output.
        out.push(format!("this := env.in{id}.Value"));
    } else if matches!(nodes[base as usize].kind, NodeKind::Func(_)) && is_void(&ret_of(nodes, n)) {
        out.push(format!("err := {call}"));
        out.push("this := struct{}{}".into());
    } else {
        out.push(format!("this, err := {call}"));
    }
    if !is_id {
        out.push("if err != nil {".into());
        out.push(format!(
            "return zero, fmt.Errorf({}, err)",
            q(&format!("{}: %w", qn(nodes, n)))
        ));
        out.push("}".into());
    }
    // The input is spent; the output is kept only for the edges that read
    // it, and let go at its last read (see go_last_reads).
    out.push(format!("zeroOut(&env.in{id})"));
    if read_nodes(graph(nodes, pid).clone())?.contains_key(&n) {
        out.push(format!("env.out{id} = this"));
    } else {
        out.push("_ = this".into());
    }
    Ok(lines(out))
}

/// `zeroOut(&env.out<N>)` for each node the transition's call reads for the
/// last time, in node order.
pub fn go_last_reads(g: GGen, pid: i64, c: Call) -> Result<List<Str>, String> {
    let nodes = &g.prog.nodes;
    let gr = graph(nodes, pid);
    let mut refs = vec![];
    for a in c.args.iter() {
        refs_of(&a.value, &mut refs);
    }
    refs.sort();
    refs.dedup();
    let mut out = vec![];
    for n in refs {
        if !live_after(gr.clone(), c.target, n)? {
            out.push(format!("zeroOut(&env.out{})", gn(nodes, n)));
        }
    }
    Ok(lines(out))
}

pub fn go_fan_out(g: GGen, pid: i64, n: i64, dim: Str) -> Result<List<Str>, String> {
    let nodes = &g.prog.nodes;
    let gr = graph(nodes, pid);
    let r = gr.opens[&n];
    let region = &gr.regions[r as usize];
    let out = gtype(&g.table, &g.tq, &ret_of(nodes, region.sink));
    Ok(lines(vec![
        format!(
            "results, err := fanOut(ctx, {dim}, func(ctx context.Context, elem {}) ({out}, error) {{",
            gtype(&g.table, &g.tq, &region.elem)
        ),
        "env := *env".into(),
        format!("return {}(ctx, &env, elem)", region_fn(nodes, pid, r)),
        "})".into(),
        "if err != nil {".into(),
        "return zero, err".into(),
        "}".into(),
        "{".into(),
        "this := results".into(),
        "_ = this".into(),
    ]))
}

pub fn go_region_head(g: GGen, pid: i64, opener: i64) -> Result<List<Str>, String> {
    let nodes = &g.prog.nodes;
    let gr = graph(nodes, pid);
    let r = gr.opens[&opener];
    let region = &gr.regions[r as usize];
    let sink = gtype(&g.table, &g.tq, &ret_of(nodes, region.sink));
    let f = region_fn(nodes, pid, r);
    Ok(lines(vec![
        format!(
            "// {f} runs one element of the for each from {}.",
            qn(nodes, opener)
        ),
        format!(
            "func {f}(ctx context.Context, env *env{}, elem {}) ({sink}, error) {{",
            gn(nodes, pid),
            gtype(&g.table, &g.tq, &region.elem)
        ),
        format!("var zero {sink}"),
        "this := elem".into(),
        "_ = this".into(),
    ]))
}

pub fn go_region_entry(g: GGen, pid: i64, opener: i64, input: Str) -> Result<List<Str>, String> {
    let nodes = &g.prog.nodes;
    let gr = graph(nodes, pid);
    let entry = gr.regions[gr.opens[&opener] as usize].entry;
    Ok(lines(vec![
        format!("env.in{} = {input}", gn(nodes, entry)),
        format!("pc := {}", pc(nodes, pid, entry)),
    ]))
}

// ---- Files ----

fn header(pkg: &str, imports: &[(String, String)]) -> String {
    let mut w = vec![
        "// Code generated by vandorf. DO NOT EDIT.".to_string(),
        "".into(),
        format!("package {pkg}"),
        "".into(),
    ];
    if !imports.is_empty() {
        w.push("import (".into());
        for (path, alias) in imports {
            w.push(if alias.is_empty() {
                q(path)
            } else {
                format!("{alias} {}", q(path))
            });
        }
        w.push(")".into());
        w.push("".into());
    }
    render(w)
}

pub fn go_header(pkg: Str, imports: List<GoImport>) -> Result<Str, String> {
    let imports: Vec<(String, String)> = imports
        .iter()
        .map(|i| (i.path.to_string(), i.alias.to_string()))
        .collect();
    Ok(header(&pkg, &imports).into())
}

pub fn go_proc_imports(imports: Map<Str, Str>) -> Result<List<GoImport>, String> {
    let mut out = vec![
        GoImport {
            path: "context".into(),
            alias: "".into(),
        },
        GoImport {
            path: "fmt".into(),
            alias: "".into(),
        },
    ];
    out.extend(imports.iter().map(|(p, a)| GoImport {
        path: p.clone(),
        alias: a.clone(),
    }));
    Ok(out.into())
}

fn file_text(pkg: &str, body: &str, mut imports: Vec<(String, String)>, setup: &GoSetup) -> String {
    // Go rejects unused imports: the types package only if the code (not
    // comments) mentions it.
    if let MaybeStr::Some(ti) = &setup.import
        && body
            .lines()
            .any(|l| !l.trim_start().starts_with("//") && l.contains(&format!("{}.", setup.pkg)))
    {
        imports.push((ti.to_string(), "".into()));
    }
    header(pkg, &imports) + body
}

pub fn go_file_text(
    pkg: Str,
    body: Str,
    imports: List<GoImport>,
    setup: GoSetup,
) -> Result<Str, String> {
    let imports = imports
        .iter()
        .map(|i| (i.path.to_string(), i.alias.to_string()))
        .collect();
    Ok(file_text(&pkg, &body, imports, &setup).into())
}

fn plain(paths: &[&str]) -> Vec<(String, String)> {
    paths
        .iter()
        .map(|p| (p.to_string(), String::new()))
        .collect()
}

fn signature(prog: &Program, t: &Table, tq: &str, n: i64, name: &str, named: bool) -> String {
    let mut s = format!("func {name}(ctx context.Context");
    for m in params_of(&prog.nodes, n).iter() {
        let p = if named { go_param(&m.name) } else { "_".into() };
        s.push_str(&format!(", {p} {}", gtype(t, tq, &m.ty)));
    }
    let ret = ret_of(&prog.nodes, n);
    if is_void(&ret) {
        s.push_str(") error");
    } else {
        s.push_str(&format!(") ({}, error)", gtype(t, tq, &ret)));
    }
    s
}

/// The signature an implementation of func n must have.
pub fn go_signature(prog: Program, t: Table, tq: Str, n: i64, name: Str) -> Result<Str, String> {
    Ok(signature(&prog, &t, &tq, n, &name, true).into())
}

pub fn go_rest_files(g: GGen, package: Str, setup: GoSetup) -> Result<List<GoFile>, String> {
    let nodes = &g.prog.nodes;
    let file = |name: &str, text: String| GoFile {
        dir: "".into(),
        name: name.into(),
        text: text.into(),
        kind: "code".into(),
    };
    let mut out = vec![];
    let mut w = vec![];
    for (id, n) in nodes.iter().enumerate() {
        let id = id as i64;
        if let NodeKind::Func(_) | NodeKind::Id(_) | NodeKind::Sig(_) = &n.kind {
            w.push(format!(
                "// {} is the input of {}.",
                in_type(nodes, id),
                qn(nodes, id)
            ));
            w.push(format!("type {} struct {{", in_type(nodes, id)));
            for m in params_of(nodes, id).iter() {
                w.push(format!(
                    "{} {}",
                    exported(&m.name),
                    gtype(&g.table, &g.tq, &m.ty)
                ));
            }
            w.push("}".into());
            w.push("".into());
        }
    }
    out.push(file(
        "inputs_vandorf.go",
        file_text(&package, &render(w), vec![], &setup),
    ));
    if !g.stubs.is_empty() {
        let mut w = vec![];
        for &n in g.stubs.keys() {
            let qual = qn(nodes, n);
            let stub = format!("unimplemented{}", gn(nodes, n));
            w.push(format!(
                "// {stub} stands in for {qual}, which names no .go implementation. Write"
            ));
            w.push(format!(
                "//\t{}",
                signature(&g.prog, &g.table, &g.tq, n, "Name", true)
            ));
            w.push("// and add `= \"file.go:Name\"` to its declaration.".into());
            w.push(format!(
                "{} {{",
                signature(&g.prog, &g.table, &g.tq, n, &stub, false)
            ));
            let msg = q(&format!("{qual} is not implemented"));
            let ret = ret_of(nodes, n);
            if is_void(&ret) {
                w.push(format!("return errors.New({msg})"));
            } else {
                w.push(format!("var zero {}", gtype(&g.table, &g.tq, &ret)));
                w.push(format!("return zero, errors.New({msg})"));
            }
            w.push("}".into());
            w.push("".into());
        }
        out.push(file(
            "stubs_vandorf.go",
            file_text(&package, &render(w), plain(&["context", "errors"]), &setup),
        ));
    }
    let mut body = GO_RUNTIME.to_string();
    for h in g.helpers.iter() {
        body.push('\n');
        body.push_str(h);
    }
    out.push(file(
        "runtime_vandorf.go",
        file_text(
            &package,
            &body,
            plain(&[
                "cmp", "context", "fmt", "os", "runtime", "slices", "sync", "unsafe",
            ]),
            &setup,
        ),
    ));
    Ok(out.into())
}

pub fn go_sort_files(files: List<GoFile>) -> Result<List<GoFile>, String> {
    let mut v = files.to_vec();
    v.sort_by(|a, b| (&a.dir, &a.name).cmp(&(&b.dir, &b.name)));
    Ok(v.into())
}

// ---- Mains ----

pub fn go_main_files(
    prog: Program,
    t: Table,
    package: Str,
    import_base: MaybeStr,
    mains: List<GoMainSpec>,
) -> Result<GoOut, String> {
    let mut files = vec![];
    for m in mains.iter() {
        match main_file(&prog, &t, &package, &import_base, m)? {
            Ok(text) => files.push(GoFile {
                dir: m.path.trim_matches('/').into(),
                name: "main_vandorf.go".into(),
                text: text.into(),
                kind: "main".into(),
            }),
            Err(e) => {
                return Ok(GoOut {
                    files: List::default(),
                    err: MaybeStr::Some(e.into()),
                });
            }
        }
    }
    Ok(GoOut {
        files: files.into(),
        err: MaybeStr::None,
    })
}

/// A package main running proc m.entry (src/codegen/go.rs: main_artifact).
fn main_file(
    prog: &Program,
    _t: &Table,
    package: &str,
    import_base: &MaybeStr,
    m: &GoMainSpec,
) -> Result<Result<String, String>, String> {
    let (name, entry) = (&m.name, &m.entry);
    let nodes = &prog.nodes;
    let Some(pid) = prog.top.iter().copied().find(|&p| {
        nodes[p as usize].qual.last() == Some(entry)
            && matches!(nodes[p as usize].kind, NodeKind::Proc(_))
    }) else {
        return Ok(Err(format!(
            "{name} runs proc {entry}, but there is no top-level proc {entry}"
        )));
    };
    let params = params_of(nodes, pid);
    let argv = params.len() == 1
        && matches!(unfold(&params[0].ty, &prog.types), Type::Array(e) if matches!(unfold(e, &prog.types), Type::Prim(p) if &**p == "string"));
    let mut typed: Vec<(String, String)> = vec![];
    if !argv {
        for p in params.iter() {
            match unfold(&p.ty, &prog.types) {
                Type::Prim(t) if &**t != "void" => typed.push((p.name.to_string(), t.to_string())),
                _ => {
                    let shown = crate::vcc::run_show_type(unfold(&p.ty, &prog.types).clone())
                        .map_err(|e| e.to_string())?;
                    return Ok(Err(format!(
                        "{entry}'s parameter {} has type {shown}; {name} reads its parameters from the command line, so they must be int, float, bool or string (or a single []string, for all the arguments)",
                        p.name
                    )));
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
    let MaybeStr::Some(import) = import_base else {
        return Ok(Err("the main package needs the code package's import path: pass --module or run inside a Go module".into()));
    };
    let alias = if package == "main" {
        "procs".to_string()
    } else {
        package.to_string()
    };
    let mut imports = plain(&["context", "fmt", "os"]);
    if typed.iter().any(|(_, p)| p != "string") {
        imports.push(("strconv".into(), "".into()));
    }
    imports.push((import.to_string(), alias.clone()));
    // The doc comment has to sit right above `package`.
    let head = header("main", &imports).replacen(
        "package main",
        &format!("// Command {name} runs proc {entry}, taking its parameters as arguments.\n//\n//\t{usage}\npackage main"),
        1,
    );
    let mut w = vec![
        format!("const usage = {}", q(&usage)),
        "".into(),
        "func main() {".into(),
    ];
    let mut args = vec!["context.Background()".to_string()];
    if argv {
        args.push("os.Args[1:]".into());
    } else {
        w.push(format!("if len(os.Args) != {} {{", typed.len() + 1));
        w.push("fmt.Fprintln(os.Stderr, usage)".into());
        w.push("os.Exit(2)".into());
        w.push("}".into());
    }
    for (i, (n, p)) in typed.iter().enumerate() {
        let var = format!("arg{}", exported(n));
        let raw = format!("os.Args[{}]", i + 1);
        let parse = match p.as_str() {
            "string" => {
                w.push(format!("{var} := {raw}"));
                args.push(var);
                continue;
            }
            "int" => format!("strconv.Atoi({raw})"),
            "float" => format!("strconv.ParseFloat({raw}, 64)"),
            _ => format!("strconv.ParseBool({raw})"),
        };
        w.push(format!("{var}, err := {parse}"));
        w.push("if err != nil {".into());
        w.push(format!(
            "fmt.Fprintf(os.Stderr, \"{n}: %v\\n%s\\n\", err, usage)"
        ));
        w.push("os.Exit(2)".into());
        w.push("}".into());
        args.push(var);
    }
    let call = format!("{alias}.Run{}({})", gn(nodes, pid), args.join(", "));
    let ret = ret_of(nodes, pid);
    if is_void(&ret) {
        w.push(format!("if err := {call}; err != nil {{"));
    } else {
        w.push(format!("result, err := {call}"));
        w.push("if err != nil {".into());
    }
    w.push("fmt.Fprintln(os.Stderr, \"error:\", err)".into());
    w.push("os.Exit(1)".into());
    w.push("}".into());
    match unfold(&ret, &prog.types) {
        Type::Prim(p) if &**p == "void" => {}
        Type::Prim(_) => w.push("fmt.Println(result)".into()),
        _ => w.push("fmt.Printf(\"%+v\\n\", result)".into()),
    }
    w.push("}".into());
    Ok(Ok(head + &render(w)))
}

const GO_RUNTIME: &str = r#"// mapGet looks up k, failing if it's missing.
func mapGet[K comparable, V any](m map[K]V, k K) (V, error) {
	v, ok := m[k]
	if !ok {
		return v, fmt.Errorf("key %v not found", k)
	}
	return v, nil
}

// sliceGet indexes xs, failing if i is out of range.
func sliceGet[E any](xs []E, i int) (E, error) {
	if i < 0 || i >= len(xs) {
		var zero E
		return zero, fmt.Errorf("index %d out of range [0, %d)", i, len(xs))
	}
	return xs[i], nil
}

func mapHas[K comparable, V any](m map[K]V, k K) bool {
	_, ok := m[k]
	return ok
}

// mapKeys returns m's keys in order.
func mapKeys[K cmp.Ordered, V any](m map[K]V) []K {
	ks := make([]K, 0, len(m))
	for k := range m {
		ks = append(ks, k)
	}
	slices.Sort(ks)
	return ks
}

// mapValues returns m's values in key order.
func mapValues[K cmp.Ordered, V any](m map[K]V) []V {
	vs := make([]V, 0, len(m))
	for _, k := range mapKeys(m) {
		vs = append(vs, m[k])
	}
	return vs
}

// mapMerge returns a new map with a's entries and b's, b winning. Maps
// are values in vandorf, so the arguments are never changed.
func mapMerge[K comparable, V any](a, b map[K]V) map[K]V {
	r := make(map[K]V, len(a)+len(b))
	for k, v := range a {
		r[k] = v
	}
	for k, v := range b {
		r[k] = v
	}
	return r
}

// Arrays that sliceConcat builds keep spare capacity in reserve. The
// reserve is hidden from every slice handed out, whose capacity is clipped
// to its length, so code that appends to one (implementations included)
// always copies. reserves records each such array by the address of its
// first element: how much of it is filled. A + whose left side ends
// exactly there appends into the reserve; any other copies. So values
// never change where anyone can see, and a loop like acc + [x] costs
// amortized O(1) per element. A finalizer drops an array's entry when it's
// freed, before its address can be reused.
type reserve struct{ filled, cap int }

var (
	reservesMu sync.Mutex
	reserves   = map[uintptr]reserve{}
)

// sliceConcat returns a new slice with a's elements, then b's; neither
// changes.
func sliceConcat[E any](a, b []E) []E {
	n := len(a) + len(b)
	var zero E
	size := int(unsafe.Sizeof(zero))
	if size == 0 {
		// All zero-size arrays share one address: nothing to reserve.
		return make([]E, n)
	}
	if p := unsafe.SliceData(a); p != nil {
		base := uintptr(unsafe.Pointer(p))
		reservesMu.Lock()
		if r, ok := reserves[base]; ok && r.filled == len(a) && n <= r.cap {
			reserves[base] = reserve{n, r.cap}
			reservesMu.Unlock()
			full := unsafe.Slice(p, r.cap)
			copy(full[len(a):n], b)
			return full[:n:n]
		}
		reservesMu.Unlock()
	}
	// Room to double, and over 16 bytes: the tiny allocator's blocks may
	// never be finalized.
	c := max(2*n, 16/size+1)
	r := make([]E, n, c)
	copy(r, a)
	copy(r[len(a):], b)
	p := unsafe.SliceData(r)
	base := uintptr(unsafe.Pointer(p))
	reservesMu.Lock()
	reserves[base] = reserve{n, c}
	reservesMu.Unlock()
	runtime.SetFinalizer(p, func(*E) {
		reservesMu.Lock()
		delete(reserves, base)
		reservesMu.Unlock()
	})
	return r[:n:n]
}

// mapRemove returns a new map without the keys ks.
func mapRemove[K comparable, V any](m map[K]V, ks []K) map[K]V {
	r := make(map[K]V, len(m))
	for k, v := range m {
		r[k] = v
	}
	for _, k := range ks {
		delete(r, k)
	}
	return r
}

// ptrTo boxes a value, for fields that would otherwise make a type contain
// itself.
func ptrTo[T any](v T) *T {
	return &v
}

// zeroOut clears a slot no later state reads, so the garbage collector can
// reclaim what it held.
func zeroOut[T any](p *T) {
	var zero T
	*p = zero
}

// joinAll runs fs concurrently and waits for all of them: a spawn's joined
// branches. The first error cancels the rest and is returned.
func joinAll(ctx context.Context, fs ...func(context.Context) error) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	var (
		wg    sync.WaitGroup
		once  sync.Once
		first error
	)
	for _, f := range fs {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if err := f(ctx); err != nil {
				once.Do(func() { first = err; cancel() })
			}
		}()
	}
	wg.Wait()
	return first
}

// spawnDetached runs f on a goroutine of its own, a spawn's detached branch:
// nothing waits for it, it outlives ctx's cancellation, and a failure is
// only reported.
func spawnDetached(ctx context.Context, f func(context.Context) error) {
	ctx = context.WithoutCancel(ctx)
	go func() {
		if err := f(ctx); err != nil {
			fmt.Fprintf(os.Stderr, "error: %v\n", err)
		}
	}()
}

// fanOut runs f over xs concurrently and returns the results in input order.
// The first error cancels the remaining elements.
func fanOut[E, R any](ctx context.Context, xs []E, f func(context.Context, E) (R, error)) ([]R, error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	out := make([]R, len(xs))
	var (
		wg    sync.WaitGroup
		once  sync.Once
		first error
	)
	for i, x := range xs {
		wg.Add(1)
		go func() {
			defer wg.Done()
			r, err := f(ctx, x)
			if err != nil {
				once.Do(func() { first = err; cancel() })
				return
			}
			out[i] = r
		}()
	}
	wg.Wait()
	return out, first
}
"#;

// ---- Spawns ----

pub fn go_spawn_lines(g: GGen, sp: SEdge, calls: List<Str>) -> Result<List<Str>, String> {
    let SEdge::Spawn(sp) = sp else {
        return err("go_spawn_lines");
    };
    // A callee error stopped the generator partway.
    if calls.len() != sp.branches.len() {
        return Ok(List::default());
    }
    let nodes = &g.prog.nodes;
    let ms = match unfold(&sp.joined, &g.prog.types) {
        Type::Struct(ms) => ms.clone(),
        _ => List::default(),
    };
    let name_of = |n: i64| nodes[n as usize].qual.last().cloned().unwrap_or_default();
    let kept = |n: i64| ms.iter().any(|m| m.name == name_of(n));
    // Void funcs return only an error.
    let err_only = |n: i64| {
        let base = base_of(nodes.clone(), n).unwrap();
        matches!(nodes[base as usize].kind, NodeKind::Func(_)) && is_void(&ret_of(nodes, n))
    };
    let wrap = |n: i64| q(&format!("{}: %w", qn(nodes, n)));
    let mut out: Vec<String> = vec![];
    for (i, b) in sp.branches.iter().enumerate().filter(|(_, b)| b.detach) {
        let n = b.call.target;
        let call = &calls[i];
        out.push("spawnDetached(ctx, func(ctx context.Context) error {".into());
        out.push(if err_only(n) {
            format!("err := {call}")
        } else {
            format!("_, err := {call}")
        });
        out.extend([
            "if err != nil {".into(),
            format!("return fmt.Errorf({}, err)", wrap(n)),
            "}".into(),
            "return nil".into(),
            "})".into(),
        ]);
    }
    let waited: Vec<usize> = (0..sp.branches.len())
        .filter(|&i| !sp.branches[i].detach)
        .collect();
    let mut fields = vec![];
    for &i in &waited {
        let n = sp.branches[i].call.target;
        if kept(n) {
            out.push(format!(
                "var r{i} {}",
                gtype(&g.table, &g.tq, &ret_of(nodes, n))
            ));
            let name = name_of(n);
            let v = if boxed(&g.table, &sp.joined, &name) {
                format!("ptrTo(r{i})")
            } else {
                format!("r{i}")
            };
            fields.push(format!("{}: {v}", exported(&name)));
        }
    }
    out.push("if err := joinAll(ctx,".into());
    for &i in &waited {
        let n = sp.branches[i].call.target;
        let call = &calls[i];
        out.push("func(ctx context.Context) error {".into());
        match (err_only(n), kept(n)) {
            (true, _) => out.push(format!("err := {call}")),
            (false, true) => {
                out.push("var err error".into());
                out.push(format!("r{i}, err = {call}"));
            }
            (false, false) => out.push(format!("_, err := {call}")),
        }
        out.extend([
            "if err != nil {".into(),
            format!("return fmt.Errorf({}, err)", wrap(n)),
            "}".into(),
            "return nil".into(),
            "},".into(),
        ]);
    }
    out.push("); err != nil {".into());
    out.push("return zero, err".into());
    out.push("}".into());
    out.push(format!(
        "this := {}{{{}}}",
        gtype(&g.table, &g.tq, &sp.joined),
        fields.join(", ")
    ));
    out.push("_ = this".into());
    Ok(lines(out))
}

// ---- HTTP servers (src/codegen/go_http.rs) ----

/// http_vandorf.go's fixed part: a copy of src/codegen/http_runtime.go.
const GO_HTTP_RUNTIME: &str = r##"// The fixed part of a generated http_vandorf.go, after http_runtime.rs:
// JSON values and their parser, encoders and decoders for primitives,
// lists and maps, and a net/http server. Generated code adds an encoder and
// decoder per type and a handler per server. Not compiled here: included
// as text by codegen::go.
//
// Requests: a route's proc takes its parameters from a JSON object body,
// or from the query string (each value read as JSON if it parses, else as
// a string). Responses: 200 and the result as JSON; 400 for a request that
// doesn't decode; 500 and {"error": ..} when the proc fails.

// httpNum is a JSON number as written.
type httpNum string

// httpObj is a JSON object, its fields in order. A JSON value is nil, a
// bool, an httpNum, a string, a []any or an httpObj.
type httpObj []httpPair

type httpPair struct {
	Key string
	Val any
}

func httpParse(s string) (any, error) {
	p := &httpParser{b: []byte(s)}
	v, err := p.value()
	if err != nil {
		return nil, err
	}
	p.ws()
	if p.i != len(p.b) {
		return nil, fmt.Errorf("unexpected input at byte %d", p.i)
	}
	return v, nil
}

func httpObject(v any, what string) (httpObj, error) {
	fs, ok := v.(httpObj)
	if !ok {
		return nil, fmt.Errorf("expected an object for %s", what)
	}
	return fs, nil
}

type httpParser struct {
	b []byte
	i int
}

func (p *httpParser) ws() {
	for p.i < len(p.b) && (p.b[p.i] == ' ' || p.b[p.i] == '\t' || p.b[p.i] == '\n' || p.b[p.i] == '\f' || p.b[p.i] == '\r') {
		p.i++
	}
}

func (p *httpParser) eat(c byte) error {
	p.ws()
	if p.i < len(p.b) && p.b[p.i] == c {
		p.i++
		return nil
	}
	return fmt.Errorf("expected '%c' at byte %d", c, p.i)
}

func (p *httpParser) value() (any, error) {
	p.ws()
	if p.i >= len(p.b) {
		return nil, errors.New("unexpected end of input")
	}
	switch c := p.b[p.i]; {
	case c == 'n':
		return p.word("null", nil)
	case c == 't':
		return p.word("true", true)
	case c == 'f':
		return p.word("false", false)
	case c == '"':
		return p.string()
	case c == '[':
		p.i++
		xs := []any{}
		p.ws()
		if p.i < len(p.b) && p.b[p.i] == ']' {
			p.i++
			return xs, nil
		}
		for {
			x, err := p.value()
			if err != nil {
				return nil, err
			}
			xs = append(xs, x)
			p.ws()
			if p.i < len(p.b) && p.b[p.i] == ',' {
				p.i++
			} else if p.i < len(p.b) && p.b[p.i] == ']' {
				p.i++
				return xs, nil
			} else {
				return nil, fmt.Errorf("expected ',' or ']' at byte %d", p.i)
			}
		}
	case c == '{':
		p.i++
		fs := httpObj{}
		p.ws()
		if p.i < len(p.b) && p.b[p.i] == '}' {
			p.i++
			return fs, nil
		}
		for {
			p.ws()
			k, err := p.string()
			if err != nil {
				return nil, err
			}
			if err := p.eat(':'); err != nil {
				return nil, err
			}
			v, err := p.value()
			if err != nil {
				return nil, err
			}
			fs = append(fs, httpPair{k, v})
			p.ws()
			if p.i < len(p.b) && p.b[p.i] == ',' {
				p.i++
			} else if p.i < len(p.b) && p.b[p.i] == '}' {
				p.i++
				return fs, nil
			} else {
				return nil, fmt.Errorf("expected ',' or '}' at byte %d", p.i)
			}
		}
	case c == '-' || (c >= '0' && c <= '9'):
		start := p.i
		p.i++
		for p.i < len(p.b) && strings.IndexByte("0123456789.eE+-", p.b[p.i]) >= 0 {
			p.i++
		}
		return httpNum(p.b[start:p.i]), nil
	}
	return nil, fmt.Errorf("unexpected character at byte %d", p.i)
}

func (p *httpParser) word(w string, v any) (any, error) {
	if !strings.HasPrefix(string(p.b[p.i:]), w) {
		return nil, fmt.Errorf("unexpected input at byte %d", p.i)
	}
	p.i += len(w)
	return v, nil
}

func (p *httpParser) string() (string, error) {
	if p.i >= len(p.b) || p.b[p.i] != '"' {
		return "", fmt.Errorf("expected a string at byte %d", p.i)
	}
	p.i++
	var out []byte
	for {
		if p.i >= len(p.b) {
			return "", errors.New("unterminated string")
		}
		c := p.b[p.i]
		switch c {
		case '"':
			p.i++
			return string(out), nil
		case '\\':
			if p.i+1 >= len(p.b) {
				return "", errors.New("unterminated string")
			}
			e := p.b[p.i+1]
			p.i += 2
			switch e {
			case 'n':
				out = append(out, '\n')
			case 't':
				out = append(out, '\t')
			case 'r':
				out = append(out, '\r')
			case 'b':
				out = append(out, 8)
			case 'f':
				out = append(out, 12)
			case 'u':
				if p.i+4 > len(p.b) {
					return "", errors.New("bad \\u escape")
				}
				cp, ok := httpHex(string(p.b[p.i : p.i+4]))
				if !ok {
					return "", errors.New("invalid digit found in string")
				}
				p.i += 4
				r := rune(cp)
				if !utf8.ValidRune(r) {
					r = utf8.RuneError
				}
				out = utf8.AppendRune(out, r)
			default:
				out = append(out, e)
			}
		default:
			out = append(out, c)
			p.i++
		}
	}
}

// httpHex reads hex digits, after an optional +, as Rust's from_str_radix.
func httpHex(s string) (uint64, bool) {
	s = strings.TrimPrefix(s, "+")
	if s == "" {
		return 0, false
	}
	var n uint64
	for i := 0; i < len(s); i++ {
		c := s[i]
		var d byte
		switch {
		case c >= '0' && c <= '9':
			d = c - '0'
		case c >= 'a' && c <= 'f':
			d = c - 'a' + 10
		case c >= 'A' && c <= 'F':
			d = c - 'A' + 10
		default:
			return 0, false
		}
		n = n*16 + uint64(d)
	}
	return n, true
}

// ---- Encoders and decoders ----

func httpWriteStr(b *strings.Builder, s string) {
	b.WriteByte('"')
	for _, c := range s {
		switch {
		case c == '"':
			b.WriteString("\\\"")
		case c == '\\':
			b.WriteString("\\\\")
		case c == '\n':
			b.WriteString("\\n")
		case c == '\r':
			b.WriteString("\\r")
		case c == '\t':
			b.WriteString("\\t")
		case c < 0x20:
			fmt.Fprintf(b, "\\u%04x", c)
		default:
			b.WriteRune(c)
		}
	}
	b.WriteByte('"')
}

func httpWriteVoid(b *strings.Builder, _ struct{}) { b.WriteString("null") }

func httpWriteInt(b *strings.Builder, x int) { b.WriteString(strconv.Itoa(x)) }

// httpWriteFloat writes x as Rust's {:?} does: the shortest digits that
// read back as x, in exponent form below 1e-4 or from 1e16 on.
func httpWriteFloat(b *strings.Builder, x float64) {
	if math.IsInf(x, 0) || math.IsNaN(x) {
		b.WriteString("null")
		return
	}
	if a := math.Abs(x); a != 0 && (a < 1e-4 || a >= 1e16) {
		s := strconv.FormatFloat(x, 'e', -1, 64)
		m, e, _ := strings.Cut(s, "e")
		n, _ := strconv.Atoi(e)
		fmt.Fprintf(b, "%se%d", m, n)
		return
	}
	s := strconv.FormatFloat(x, 'f', -1, 64)
	if !strings.Contains(s, ".") {
		s += ".0"
	}
	b.WriteString(s)
}

func httpWriteBool(b *strings.Builder, x bool) {
	if x {
		b.WriteString("true")
	} else {
		b.WriteString("false")
	}
}

func httpWriteString(b *strings.Builder, x string) { httpWriteStr(b, x) }

// httpListW encodes a list with f.
func httpListW[T any](f func(*strings.Builder, T)) func(*strings.Builder, []T) {
	return func(b *strings.Builder, xs []T) {
		b.WriteByte('[')
		for i, x := range xs {
			if i > 0 {
				b.WriteByte(',')
			}
			f(b, x)
		}
		b.WriteByte(']')
	}
}

// httpMapW encodes a map as an object, keys in order.
func httpMapW[K cmp.Ordered, V any](key func(K) string, f func(*strings.Builder, V)) func(*strings.Builder, map[K]V) {
	return func(b *strings.Builder, m map[K]V) {
		ks := make([]K, 0, len(m))
		for k := range m {
			ks = append(ks, k)
		}
		slices.Sort(ks)
		b.WriteByte('{')
		for i, k := range ks {
			if i > 0 {
				b.WriteByte(',')
			}
			httpWriteStr(b, key(k))
			b.WriteByte(':')
			f(b, m[k])
		}
		b.WriteByte('}')
	}
}

func httpIntKey(k int) string    { return strconv.Itoa(k) }
func httpStringKey(k string) string { return k }

func httpReadVoid(any) (struct{}, error) { return struct{}{}, nil }

func httpReadInt(v any) (int, error) {
	n, ok := v.(httpNum)
	if !ok {
		return 0, errors.New("expected an int")
	}
	x, err := strconv.ParseInt(string(n), 10, 64)
	if err != nil {
		return 0, fmt.Errorf("%s is not an int", n)
	}
	return int(x), nil
}

func httpReadFloat(v any) (float64, error) {
	n, ok := v.(httpNum)
	if !ok {
		return 0, errors.New("expected a float")
	}
	// Out of range reads as infinity or zero, as in Rust.
	x, err := strconv.ParseFloat(string(n), 64)
	if err != nil && !errors.Is(err, strconv.ErrRange) {
		return 0, fmt.Errorf("%s is not a float", n)
	}
	return x, nil
}

func httpReadBool(v any) (bool, error) {
	x, ok := v.(bool)
	if !ok {
		return false, errors.New("expected a bool")
	}
	return x, nil
}

func httpReadString(v any) (string, error) {
	x, ok := v.(string)
	if !ok {
		return "", errors.New("expected a string")
	}
	return x, nil
}

// httpListR decodes a list with f.
func httpListR[T any](f func(any) (T, error)) func(any) ([]T, error) {
	return func(v any) ([]T, error) {
		xs, ok := v.([]any)
		if !ok {
			return nil, errors.New("expected an array")
		}
		out := make([]T, 0, len(xs))
		for _, x := range xs {
			y, err := f(x)
			if err != nil {
				return nil, err
			}
			out = append(out, y)
		}
		return out, nil
	}
}

// httpMapR decodes an object as a map.
func httpMapR[K cmp.Ordered, V any](key func(string) (K, error), f func(any) (V, error)) func(any) (map[K]V, error) {
	return func(v any) (map[K]V, error) {
		fs, err := httpObject(v, "a map")
		if err != nil {
			return nil, err
		}
		m := make(map[K]V, len(fs))
		for _, kv := range fs {
			k, err := key(kv.Key)
			if err != nil {
				return nil, err
			}
			x, err := f(kv.Val)
			if err != nil {
				return nil, err
			}
			m[k] = x
		}
		return m, nil
	}
}

func httpIntFromKey(k string) (int, error) {
	x, err := strconv.ParseInt(k, 10, 64)
	if err != nil {
		return 0, fmt.Errorf("map key %s is not an int", k)
	}
	return int(x), nil
}

func httpStringFromKey(k string) (string, error) { return k, nil }

func httpField(fs httpObj, name string) (any, error) {
	for _, kv := range fs {
		if kv.Key == name {
			return kv.Val, nil
		}
	}
	return nil, fmt.Errorf("missing field %s", name)
}

// httpArg is a route's parameter, decoded; a failure is a 400.
func httpArg[T any](params any, name string, f func(any) (T, error)) (T, error) {
	var zero T
	fs, err := httpObject(params, "the parameters")
	if err != nil {
		return zero, err
	}
	for _, kv := range fs {
		if kv.Key == name {
			x, err := f(kv.Val)
			if err != nil {
				return zero, fmt.Errorf("parameter %s: %s", name, err)
			}
			return x, nil
		}
	}
	return zero, fmt.Errorf("missing parameter %s", name)
}

func httpJSON[T any](f func(*strings.Builder, T), x T) string {
	var b strings.Builder
	f(&b, x)
	return b.String()
}

// ---- The server ----

// HTTPRequest is a request: header names lowercased, the query string
// undecoded.
type HTTPRequest struct {
	Method  string
	Path    string
	Query   string
	Headers map[string]string
	Body    string
}

type HTTPResponse struct {
	Status  int
	Headers map[string]string
	Body    string
}

// HTTPHandler is a server's routes: the response, and whether it has the
// route.
type HTTPHandler func(ctx context.Context, req *HTTPRequest) (HTTPResponse, bool)

// HTTPStatic is a file served as it is.
type HTTPStatic struct {
	Path string
	Type string
	Body string
}

func httpJSONResponse(status int, body string) HTTPResponse {
	return HTTPResponse{Status: status, Headers: map[string]string{"Content-Type": "application/json"}, Body: body}
}

func httpErrorResponse(status int, msg string) HTTPResponse {
	var b strings.Builder
	b.WriteString("{\"error\":")
	httpWriteStr(&b, msg)
	b.WriteByte('}')
	return httpJSONResponse(status, b.String())
}

// httpQueryPairs is the query string's parameters, decoded.
func httpQueryPairs(query string) [][2]string {
	var out [][2]string
	for _, kv := range strings.Split(query, "&") {
		if kv == "" {
			continue
		}
		k, v, _ := strings.Cut(kv, "=")
		out = append(out, [2]string{httpPercentDecode(k), httpPercentDecode(v)})
	}
	return out
}

func httpQueryMap(query string) map[string]string {
	m := map[string]string{}
	for _, kv := range httpQueryPairs(query) {
		m[kv[0]] = kv[1]
	}
	return m
}

// httpJSONRoute is a JSON route: its parameters from a JSON object body,
// or the query string (each value read as JSON if it parses, else as a
// string).
func httpJSONRoute(ctx context.Context, req *HTTPRequest, call func(context.Context, any) (string, int, error)) HTTPResponse {
	var params any
	if strings.TrimSpace(req.Body) == "" {
		fs := httpObj{}
		for _, kv := range httpQueryPairs(req.Query) {
			v, err := httpParse(kv[1])
			if err != nil {
				v = kv[1]
			}
			fs = append(fs, httpPair{kv[0], v})
		}
		params = fs
	} else {
		v, err := httpParse(req.Body)
		if err != nil {
			return httpErrorResponse(400, "body: "+err.Error())
		}
		params = v
	}
	text, status, err := call(ctx, params)
	if err != nil {
		return httpErrorResponse(status, err.Error())
	}
	return httpJSONResponse(200, text)
}

// HTTPRun is a server's main: [--addr host:port] from the command line
// (else addr, the manifest's), then serves.
func HTTPRun(addr string, handler HTTPHandler, statics []HTTPStatic) {
	if a, ok := httpListenAddr(addr); ok {
		httpServe(a, handler, statics)
	}
}

// HTTPRunSetup is a server's main with a setup: the address as for
// HTTPRun, then the setup (if it fails, the server exits without
// listening), then serves.
func HTTPRunSetup(addr string, setup func(context.Context) error, handler HTTPHandler, statics []HTTPStatic) {
	a, ok := httpListenAddr(addr)
	if !ok {
		return
	}
	if err := setup(context.Background()); err != nil {
		fmt.Fprintf(os.Stderr, "error: setup: %v\n", err)
		os.Exit(1)
	}
	httpServe(a, handler, statics)
}

// httpListenAddr is --addr host:port from the command line, else addr;
// false after printing the usage for --help.
func httpListenAddr(addr string) (string, bool) {
	name := os.Args[0][strings.LastIndex(os.Args[0], "/")+1:]
	usage := fmt.Sprintf("usage: %s [--addr host:port]   (default %s)", name, addr)
	args := os.Args[1:]
	for i := 0; i < len(args); i += 2 {
		a := args[i]
		switch {
		case a == "--addr" && i+1 < len(args):
			addr = args[i+1]
		case a == "-h" || a == "--help":
			fmt.Fprintln(os.Stderr, usage)
			return "", false
		default:
			fmt.Fprintf(os.Stderr, "error: unexpected %s\n%s\n", a, usage)
			os.Exit(2)
		}
	}
	return addr, true
}

// httpServe serves until the process is stopped.
func httpServe(addr string, handler HTTPHandler, statics []HTTPStatic) {
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: cannot listen on %s: %v\n", addr, err)
		os.Exit(1)
	}
	fmt.Fprintf(os.Stderr, "listening on http://%s\n", addr)
	http.Serve(ln, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(io.LimitReader(r.Body, 64<<20))
		path, query, _ := strings.Cut(r.RequestURI, "?")
		headers := map[string]string{}
		for k, vs := range r.Header {
			if len(vs) > 0 {
				headers[strings.ToLower(k)] = vs[len(vs)-1]
			}
		}
		if r.Host != "" {
			headers["host"] = r.Host
		}
		req := &HTTPRequest{Method: r.Method, Path: path, Query: query, Headers: headers, Body: strings.ToValidUTF8(string(body), "�")}
		resp := httpRespond(r.Context(), req, handler, statics)
		if resp.Status < 100 || resp.Status > 999 {
			resp = httpErrorResponse(500, fmt.Sprintf("status %d is not an HTTP status", resp.Status))
		}
		for k, v := range resp.Headers {
			w.Header()[k] = []string{v}
		}
		// Send no Content-Type the response doesn't set.
		if _, ok := w.Header()["Content-Type"]; !ok {
			w.Header()["Content-Type"] = nil
		}
		w.WriteHeader(resp.Status)
		io.WriteString(w, resp.Body)
	}))
}

func httpRespond(ctx context.Context, req *HTTPRequest, handler HTTPHandler, statics []HTTPStatic) HTTPResponse {
	if req.Method == "GET" {
		for _, s := range statics {
			if s.Path == req.Path {
				return HTTPResponse{Status: 200, Headers: map[string]string{"Content-Type": s.Type}, Body: s.Body}
			}
		}
	}
	if resp, ok := handler(ctx, req); ok {
		return resp
	}
	return httpErrorResponse(404, fmt.Sprintf("no route %s %s", req.Method, req.Path))
}

func httpPercentDecode(s string) string {
	var out []byte
	for i := 0; i < len(s); i++ {
		switch c := s[i]; {
		case c == '+':
			out = append(out, ' ')
		case c == '%' && i+2 < len(s):
			if x, ok := httpHex(s[i+1 : i+3]); ok {
				out = append(out, byte(x))
				i += 2
			} else {
				out = append(out, '%')
			}
		default:
			out = append(out, c)
		}
	}
	return strings.ToValidUTF8(string(out), "�")
}
"##;

/// What http_vandorf.go imports, besides the types package.
const GO_HTTP_IMPORTS: &[&str] = &[
    "cmp",
    "context",
    "errors",
    "fmt",
    "io",
    "math",
    "net",
    "net/http",
    "os",
    "slices",
    "strconv",
    "strings",
    "unicode/utf8",
];

/// A server's name as part of a Go identifier: `web-api` -> `Web_api`.
fn go_server_ident(name: &str) -> String {
    exported(name)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A Go string literal for any text: control characters escaped too.
fn go_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn go_prim_name(p: &str) -> &'static str {
    match p {
        "void" => "Void",
        "int" => "Int",
        "float" => "Float",
        "bool" => "Bool",
        _ => "String",
    }
}

fn go_key_name(k: &TyRef) -> &'static str {
    match k {
        TyRef::Prim(p) if &**p == "int" => "Int",
        _ => "String",
    }
}

struct GoHttp<'a> {
    prog: &'a Program,
    t: &'a Table,
    tq: &'a str,
    w: Vec<Line>,
}

impl GoHttp<'_> {
    fn line(&mut self, s: String) {
        self.w.push(Line::L(s.into()));
    }

    fn go_type(&self, ty: &Type) -> String {
        gtype(self.t, self.tq, ty)
    }

    fn tyref(&self, ty: &Type) -> TyRef {
        get(self.t.clone(), ty.clone()).unwrap()
    }

    fn enc(&self, r: &TyRef) -> String {
        match r {
            TyRef::Prim(p) => format!("httpWrite{}", go_prim_name(p)),
            TyRef::Array(e) => format!("httpListW({})", self.enc(e)),
            TyRef::Map(m) => format!(
                "httpMapW(http{}Key, {})",
                go_key_name(&m.key),
                self.enc(&m.value)
            ),
            TyRef::Named(i) => format!("httpEnc{}", self.t.types[*i as usize].name),
        }
    }

    fn dec(&self, r: &TyRef) -> String {
        match r {
            TyRef::Prim(p) => format!("httpRead{}", go_prim_name(p)),
            TyRef::Array(e) => format!("httpListR({})", self.dec(e)),
            TyRef::Map(m) => format!(
                "httpMapR(http{}FromKey, {})",
                go_key_name(&m.key),
                self.dec(&m.value)
            ),
            TyRef::Named(i) => format!("httpDec{}", self.t.types[*i as usize].name),
        }
    }

    fn is_boxed(&self, i: usize, n: &str) -> bool {
        self.t.boxed.contains_key(&Str::from(format!("{i}:{n}")))
    }

    /// An encoder and a decoder per named type.
    fn codecs(&mut self) {
        let t = self.t.clone();
        for (i, nt) in t.types.iter().enumerate() {
            let ty = format!("{}{}", self.tq, nt.name);
            match &nt.shape {
                Shape::Struct(fs) => {
                    self.line(format!(
                        "func httpEnc{}(b *strings.Builder, x {ty}) {{",
                        nt.name
                    ));
                    self.line("b.WriteByte('{')".into());
                    for (j, f) in fs.iter().enumerate() {
                        let key = format!("{}{}:", if j > 0 { "," } else { "" }, q(&f.name));
                        self.line(format!("b.WriteString({})", q(&key)));
                        let v = if self.is_boxed(i, &f.name) {
                            format!("*x.{}", exported(&f.name))
                        } else {
                            format!("x.{}", exported(&f.name))
                        };
                        let e = self.enc(&f.r);
                        self.line(format!("{e}(b, {v})"));
                    }
                    self.line("b.WriteByte('}')".into());
                    self.line("}".into());
                    self.line("".into());
                    self.line(format!("func httpDec{}(v any) ({ty}, error) {{", nt.name));
                    self.line(format!("var x {ty}"));
                    self.line(format!(
                        "{}, err := httpObject(v, {})",
                        if fs.is_empty() { "_" } else { "fs" },
                        q(&nt.name)
                    ));
                    self.line("if err != nil {".into());
                    self.line("return x, err".into());
                    self.line("}".into());
                    for f in fs.iter() {
                        self.line(format!(
                            "if f, err := httpField(fs, {}); err != nil {{",
                            q(&f.name)
                        ));
                        self.line("return x, err".into());
                        let d = self.dec(&f.r);
                        self.line(format!("}} else if y, err := {d}(f); err != nil {{"));
                        self.line(format!(
                            "return x, errors.New({} + err.Error())",
                            q(&format!("{}: ", f.name))
                        ));
                        self.line("} else {".into());
                        let y = if self.is_boxed(i, &f.name) { "&y" } else { "y" };
                        self.line(format!("x.{} = {y}", exported(&f.name)));
                        self.line("}".into());
                    }
                    self.line("return x, nil".into());
                    self.line("}".into());
                    self.line("".into());
                }
                Shape::Enum(vs) => {
                    let void = |r: &TyRef| matches!(r, TyRef::Prim(p) if &**p == "void");
                    self.line(format!(
                        "func httpEnc{}(b *strings.Builder, x {ty}) {{",
                        nt.name
                    ));
                    self.line("switch x.Tag {".into());
                    for v in vs.iter() {
                        self.line(format!("case {}:", q(&v.name)));
                        if void(&v.r) {
                            self.line(format!("b.WriteString({})", q(&q(&v.name))));
                        } else {
                            self.line(format!(
                                "b.WriteString({})",
                                q(&format!("{{{}:", q(&v.name)))
                            ));
                            let x = if self.is_boxed(i, &v.name) {
                                format!("*x.{}", exported(&v.name))
                            } else {
                                format!("x.{}", exported(&v.name))
                            };
                            let e = self.enc(&v.r);
                            self.line(format!("{e}(b, {x})"));
                            self.line("b.WriteByte('}')".into());
                        }
                    }
                    self.line("default:".into());
                    self.line("b.WriteString(\"null\")".into());
                    self.line("}".into());
                    self.line("}".into());
                    self.line("".into());
                    self.line(format!("func httpDec{}(v any) ({ty}, error) {{", nt.name));
                    self.line(format!("var x {ty}"));
                    self.line("switch v := v.(type) {".into());
                    self.line("case string:".into());
                    self.line("switch v {".into());
                    for v in vs.iter().filter(|v| void(&v.r)) {
                        self.line(format!("case {}:", q(&v.name)));
                        self.line(format!("x.Tag = {}", q(&v.name)));
                        self.line("return x, nil".into());
                    }
                    self.line("}".into());
                    self.line(format!(
                        "return x, fmt.Errorf({}, v)",
                        q(&format!("{} has no variant %s without a payload", nt.name))
                    ));
                    self.line("case httpObj:".into());
                    self.line("if len(v) == 1 {".into());
                    self.line("switch v[0].Key {".into());
                    for v in vs.iter().filter(|v| !void(&v.r)) {
                        self.line(format!("case {}:", q(&v.name)));
                        let d = self.dec(&v.r);
                        self.line(format!("y, err := {d}(v[0].Val)"));
                        self.line("if err != nil {".into());
                        self.line("return x, err".into());
                        self.line("}".into());
                        self.line(format!("x.Tag = {}", q(&v.name)));
                        let y = if self.is_boxed(i, &v.name) { "&y" } else { "y" };
                        self.line(format!("x.{} = {y}", exported(&v.name)));
                        self.line("return x, nil".into());
                    }
                    self.line("}".into());
                    self.line(format!(
                        "return x, fmt.Errorf({}, v[0].Key)",
                        q(&format!("{} has no variant %s with a payload", nt.name))
                    ));
                    self.line("}".into());
                    self.line("}".into());
                    self.line(format!(
                        "return x, errors.New({})",
                        q(&format!(
                            "expected a variant of {}: \"Tag\" or {{\"Tag\": payload}}",
                            nt.name
                        ))
                    ));
                    self.line("}".into());
                    self.line("".into());
                }
            }
        }
    }

    /// A JSON route's call; with a setup (its static and server), the
    /// context is its ctx parameter.
    fn call(&mut self, fname: &str, pid: i64, p: &str, ctx: Option<(&str, &str)>) {
        let nodes = self.prog.nodes.clone();
        match ctx {
            Some((_, server)) => self.line(format!(
                "// {fname} runs proc {p} with the request's parameters and http server {server}'s context."
            )),
            None => self.line(format!(
                "// {fname} runs proc {p} with the request's parameters."
            )),
        }
        self.line(format!(
            "func {fname}(ctx context.Context, params any) (string, int, error) {{"
        ));
        let mut args = vec!["ctx".to_string()];
        for m in params_of(&nodes, pid).iter() {
            let var = format!("arg{}", exported(&m.name));
            match ctx {
                Some((stat, server)) if &*m.name == "ctx" => {
                    self.line(format!("if !{stat}Set {{"));
                    self.line(format!(
                        "return \"\", 500, errors.New({})",
                        q(&format!("http server {server} is not set up"))
                    ));
                    self.line("}".into());
                    self.line(format!("{var} := {stat}"));
                }
                _ => {
                    let d = self.dec(&self.tyref(&m.ty));
                    self.line(format!(
                        "{var}, err := httpArg(params, {}, {d})",
                        q(&m.name)
                    ));
                    self.line("if err != nil {".into());
                    self.line("return \"\", 400, err".into());
                    self.line("}".into());
                }
            }
            args.push(var);
        }
        self.line(format!(
            "result, err := Run{}({})",
            gn(&nodes, pid),
            args.join(", ")
        ));
        self.line("if err != nil {".into());
        self.line("return \"\", 500, err".into());
        self.line("}".into());
        let e = self.enc(&self.tyref(&ret_of(&nodes, pid)));
        self.line(format!("return httpJSON({e}, result), 200, nil"));
        self.line("}".into());
        self.line("".into());
    }

    /// A raw route's call: the request as the proc's request struct, and
    /// with a setup the context as ctx.
    fn raw(&mut self, fname: &str, pid: i64, p: &str, req_ty: &Type, ctx: Option<(&str, &str)>) {
        let nodes = self.prog.nodes.clone();
        match ctx {
            Some((_, server)) => self.line(format!(
                "// {fname} runs proc {p} on the raw request with http server {server}'s context."
            )),
            None => self.line(format!("// {fname} runs proc {p} on the raw request.")),
        }
        self.line(format!(
            "func {fname}(ctx context.Context, req *HTTPRequest) HTTPResponse {{"
        ));
        self.line(format!(
            "r := {}{{Method: req.Method, Path: req.Path, Query: httpQueryMap(req.Query), Headers: req.Headers, Body: req.Body}}",
            self.go_type(req_ty)
        ));
        let mut args = vec!["ctx".to_string()];
        if let Some((stat, server)) = ctx {
            self.line(format!("if !{stat}Set {{"));
            self.line(format!(
                "return httpErrorResponse(500, {})",
                q(&format!("http server {server} is not set up"))
            ));
            self.line("}".into());
            args.extend(params_of(&nodes, pid).iter().map(|m| {
                if &*m.name == "ctx" {
                    stat.to_string()
                } else {
                    "r".into()
                }
            }));
        } else {
            args.push("r".into());
        }
        self.line(format!(
            "res, err := Run{}({})",
            gn(&nodes, pid),
            args.join(", ")
        ));
        self.line("if err != nil {".into());
        self.line("return httpErrorResponse(500, err.Error())".into());
        self.line("}".into());
        self.line(
            "return HTTPResponse{Status: res.Status, Headers: res.Headers, Body: res.Body}".into(),
        );
        self.line("}".into());
        self.line("".into());
    }

    fn find(&self, server: &str, p: &str) -> Result<i64, String> {
        match find_top(self.prog.clone(), p.into())? {
            -1 => Err(format!(
                "http server {server} runs proc {p}, but there is no top-level proc {p}"
            )),
            pid => Ok(pid),
        }
    }

    /// A server with a setup proc: its context, HTTPSetup<Server>, and a
    /// call per route and raw route passing the context as ctx.
    fn setup_calls(&mut self, srv: &GoServerSpec) -> Result<(), String> {
        let nodes = self.prog.nodes.clone();
        let (name, setup) = (&*srv.name, &*srv.setup);
        let sid = match find_top(self.prog.clone(), setup.into())? {
            -1 => {
                return Err(format!(
                    "http server {name}'s setup runs proc {setup}, but there is no top-level proc {setup}"
                ));
            }
            sid => sid,
        };
        if !params_of(&nodes, sid).is_empty() {
            return Err(format!(
                "http server {name}: setup proc {setup} must take no parameters"
            ));
        }
        let ctx_ty = ret_of(&nodes, sid);
        let ident = go_server_ident(name);
        let (gctx, stat) = (self.go_type(&ctx_ty), format!("httpCtx{ident}"));
        // Each of the server's procs must take ctx, of the setup's type.
        let check = |me: &Self, kind: &str, p: &str, pid: i64| -> Result<(), String> {
            match params_of(&nodes, pid).iter().find(|m| &*m.name == "ctx") {
                None => Err(format!(
                    "http server {name}: {kind} proc {p} must take ctx: {}, its setup proc {setup}'s result",
                    show_type(&ctx_ty)
                )),
                Some(m) if me.go_type(&m.ty) != gctx => Err(format!(
                    "http server {name}: {kind} proc {p} takes ctx: {}, but its setup proc {setup} returns {}",
                    show_type(&m.ty),
                    show_type(&ctx_ty)
                )),
                Some(_) => Ok(()),
            }
        };
        self.line(format!(
            "// {stat} is http server {name}'s context: proc {setup}'s result, set before serving."
        ));
        self.line(format!("var {stat} {gctx}"));
        self.line(format!("var {stat}Set bool"));
        self.line("".into());
        self.line(format!(
            "// HTTPSetup{ident} runs proc {setup}, the setup of http server {name}."
        ));
        self.line(format!(
            "func HTTPSetup{ident}(ctx context.Context) error {{"
        ));
        self.line(format!("c, err := Run{}(ctx)", gn(&nodes, sid)));
        self.line("if err != nil {".into());
        self.line("return err".into());
        self.line("}".into());
        self.line(format!("{stat}, {stat}Set = c, true"));
        self.line("return nil".into());
        self.line("}".into());
        self.line("".into());
        let mut procs: Vec<&str> = srv.routes.iter().map(|r| &*r.proc).collect();
        procs.sort();
        procs.dedup();
        for p in procs {
            let pid = self.find(name, p)?;
            check(self, "route", p, pid)?;
            self.call(
                &format!("httpCall{ident}{}", gn(&nodes, pid)),
                pid,
                p,
                Some((&stat, name)),
            );
        }
        let mut raws: Vec<&str> = srv.raw.iter().map(|r| &*r.proc).collect();
        raws.sort();
        raws.dedup();
        for p in raws {
            let pid = self.find(name, p)?;
            check(self, "raw route", p, pid)?;
            let rest: Vec<Member> = params_of(&nodes, pid)
                .iter()
                .filter(|m| &*m.name != "ctx")
                .cloned()
                .collect();
            if !(rest.len() == 1
                && raw_shape(self.prog, &rest[0].ty, RAW_REQUEST)
                && raw_shape(self.prog, &ret_of(&nodes, pid), RAW_RESPONSE))
            {
                return Err(format!(
                    "http server {name}: raw route proc {p} must take ctx and one {}, and return {}",
                    show_shape(RAW_REQUEST),
                    show_shape(RAW_RESPONSE)
                ));
            }
            self.raw(
                &format!("httpRaw{ident}{}", gn(&nodes, pid)),
                pid,
                p,
                &rest[0].ty,
                Some((&stat, name)),
            );
        }
        Ok(())
    }
}

fn go_http_body(
    prog: &Program,
    t: &Table,
    tq: &str,
    servers: &List<GoServerSpec>,
) -> Result<String, String> {
    let nodes = prog.nodes.clone();
    let mut h = GoHttp {
        prog,
        t,
        tq,
        w: vec![Line::Raw(GO_HTTP_RUNTIME.into())],
    };
    h.line("".into());
    h.codecs();
    // One call per proc a route runs (servers with a setup get their own).
    let plain: Vec<&GoServerSpec> = servers.iter().filter(|s| s.setup.is_empty()).collect();
    let mut procs: Vec<&str> = plain
        .iter()
        .flat_map(|s| s.routes.iter().map(|r| &*r.proc))
        .collect();
    procs.sort();
    procs.dedup();
    for p in procs {
        let srv = plain
            .iter()
            .find(|s| s.routes.iter().any(|r| &*r.proc == p))
            .unwrap();
        let pid = h.find(&srv.name, p)?;
        h.call(&format!("httpCall{}", gn(&nodes, pid)), pid, p, None);
    }
    // One raw call per proc a raw route runs.
    let mut raws: Vec<&str> = plain
        .iter()
        .flat_map(|s| s.raw.iter().map(|r| &*r.proc))
        .collect();
    raws.sort();
    raws.dedup();
    for p in raws {
        let srv = plain
            .iter()
            .find(|s| s.raw.iter().any(|r| &*r.proc == p))
            .unwrap();
        let pid = h.find(&srv.name, p)?;
        let params = params_of(&nodes, pid);
        let ok = params.len() == 1
            && raw_shape(prog, &params[0].ty, RAW_REQUEST)
            && raw_shape(prog, &ret_of(&nodes, pid), RAW_RESPONSE);
        if !ok {
            return Err(format!(
                "http server {}: raw route proc {p} must take one {} and return {}",
                srv.name,
                show_shape(RAW_REQUEST),
                show_shape(RAW_RESPONSE)
            ));
        }
        h.raw(
            &format!("httpRaw{}", gn(&nodes, pid)),
            pid,
            p,
            &params[0].ty,
            None,
        );
    }
    for srv in servers.iter().filter(|s| !s.setup.is_empty()) {
        h.setup_calls(srv)?;
    }
    for srv in servers.iter() {
        let ident = go_server_ident(&srv.name);
        // A server with a setup has calls of its own.
        let own = if srv.setup.is_empty() {
            String::new()
        } else {
            ident.clone()
        };
        h.line(format!(
            "// HTTPHandle{ident} routes the requests of http server {}.",
            srv.name
        ));
        h.line(format!(
            "func HTTPHandle{ident}(ctx context.Context, req *HTTPRequest) (HTTPResponse, bool) {{"
        ));
        h.line("switch req.Method + \" \" + req.Path {".into());
        // Two spellings of one route: the first is taken.
        let mut seen = std::collections::HashSet::new();
        for r in srv.routes.iter() {
            let key = format!("{} {}", r.method, r.path);
            if seen.insert(key.clone()) {
                let pid = find_top(prog.clone(), r.proc.clone())?;
                h.line(format!("case {}:", q(&key)));
                h.line(format!(
                    "return httpJSONRoute(ctx, req, httpCall{own}{}), true",
                    gn(&nodes, pid)
                ));
            }
        }
        for r in srv.raw.iter() {
            let key = format!("{} {}", r.method, r.path);
            if seen.insert(key.clone()) {
                let pid = find_top(prog.clone(), r.proc.clone())?;
                h.line(format!("case {}:", q(&key)));
                h.line(format!(
                    "return httpRaw{own}{}(ctx, req), true",
                    gn(&nodes, pid)
                ));
            }
        }
        h.line("}".into());
        h.line("return HTTPResponse{}, false".into());
        h.line("}".into());
        h.line("".into());
    }
    Ok(render_lines(h.w.into(), "\t".into())?.to_string())
}

/// http_vandorf.go, if there are servers.
pub fn go_http_files(
    prog: Program,
    t: Table,
    package: Str,
    setup: GoSetup,
    servers: List<GoServerSpec>,
) -> Result<GoOut, String> {
    if servers.is_empty() {
        return Ok(GoOut {
            files: List::default(),
            err: MaybeStr::None,
        });
    }
    Ok(match go_http_body(&prog, &t, &setup.tq, &servers) {
        Ok(body) => {
            let imports = GO_HTTP_IMPORTS
                .iter()
                .map(|i| (i.to_string(), String::new()))
                .collect();
            GoOut {
                files: vec![GoFile {
                    dir: "".into(),
                    name: "http_vandorf.go".into(),
                    text: file_text(&package, &body, imports, &setup).into(),
                    kind: "code".into(),
                }]
                .into(),
                err: MaybeStr::None,
            }
        }
        Err(e) => GoOut {
            files: List::default(),
            err: MaybeStr::Some(e.into()),
        },
    })
}

/// Each server's package main: its static files, then HTTPRun (or
/// HTTPRunSetup) with its handler.
pub fn go_server_mains(
    package: Str,
    import_base: MaybeStr,
    servers: List<GoServerSpec>,
) -> Result<GoOut, String> {
    let mut files = vec![];
    for srv in servers.iter() {
        let MaybeStr::Some(import) = &import_base else {
            return Ok(GoOut {
                files: List::default(),
                err: MaybeStr::Some(
                    "the main package needs the code package's import path: pass --module or run inside a Go module"
                        .into(),
                ),
            });
        };
        let alias = if &*package == "main" {
            "procs".to_string()
        } else {
            package.to_string()
        };
        let ident = go_server_ident(&srv.name);
        let mut doc = vec![format!(
            "// Command {} serves http server {} on {} (or --addr host:port):",
            srv.name, srv.name, srv.addr
        )];
        for r in srv.routes.iter() {
            doc.push(format!(
                "//   - {} {} runs proc {}",
                r.method, r.path, r.proc
            ));
        }
        for r in srv.raw.iter() {
            doc.push(format!(
                "//   - {} {} runs proc {} on the raw request",
                r.method, r.path, r.proc
            ));
        }
        for s in srv.statics.iter() {
            doc.push(format!("//   - GET {} serves {}", s.url, s.file));
        }
        if !srv.setup.is_empty() {
            doc.push("//".into());
            doc.push(format!(
                "// Proc {} runs first; its result is each route's ctx.",
                srv.setup
            ));
        }
        let head = header("main", &[(import.to_string(), alias.clone())]).replacen(
            "package main",
            &format!("{}\npackage main", doc.join("\n")),
            1,
        );
        let mut w = vec![];
        if srv.statics.is_empty() {
            w.push(format!("var statics []{alias}.HTTPStatic"));
        } else {
            w.push(format!("var statics = []{alias}.HTTPStatic{{"));
            for s in srv.statics.iter() {
                w.push(format!(
                    "{{Path: {}, Type: {}, Body: {}}},",
                    q(&s.url),
                    q(&s.ctype),
                    go_quote(&s.text)
                ));
            }
            w.push("}".into());
        }
        w.push("".into());
        w.push("func main() {".into());
        if srv.setup.is_empty() {
            w.push(format!(
                "{alias}.HTTPRun({}, {alias}.HTTPHandle{ident}, statics)",
                q(&srv.addr)
            ));
        } else {
            w.push(format!(
                "{alias}.HTTPRunSetup({}, {alias}.HTTPSetup{ident}, {alias}.HTTPHandle{ident}, statics)",
                q(&srv.addr)
            ));
        }
        w.push("}".into());
        files.push(GoFile {
            dir: srv.path.trim_matches('/').into(),
            name: "main_vandorf.go".into(),
            text: (head + &render(w)).into(),
            kind: "main".into(),
        });
    }
    Ok(GoOut {
        files: files.into(),
        err: MaybeStr::None,
    })
}
