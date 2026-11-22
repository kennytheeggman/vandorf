//! Leaves for vcc's Go backend (go.sys): names, types and the text around
//! the procs' state machines, after src/codegen/go.rs.

use super::gen_leaf::{
    base_of, get, gname, join_path, member_of, quote, render_lines, rs_impl, ty_boxed,
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
        NodeKind::Func(_) | NodeKind::Id(_) => Ok(format!("in{}", gn(&nodes, n)).into()),
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
    let is_id = matches!(nodes[n as usize].kind, NodeKind::Id(_));
    if is_id {
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
    out.push(format!("env.out{id} = this"));
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
        if let NodeKind::Func(_) | NodeKind::Id(_) = &n.kind {
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
            plain(&["cmp", "context", "fmt", "os", "slices", "sync"]),
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

// sliceConcat returns a new slice with a's elements, then b's.
func sliceConcat[E any](a, b []E) []E {
	r := make([]E, 0, len(a)+len(b))
	return append(append(r, a...), b...)
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
