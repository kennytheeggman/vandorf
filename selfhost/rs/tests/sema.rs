//! vcc's sema (../sys/sema) against the reference (src/sema.rs, src/types.rs).

use std::sync::Arc;
use vandorf::types as r;
use vcc::vcc::types as v;

/// The projects and single files in the repo, as the reference sees them.
fn programs() -> Vec<(String, vandorf::sema::Program)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = vec![];
    for dir in ["selfhost", "examples/demo"] {
        let p = vandorf::project::load(&root.join(dir).join("vandorf.toml")).unwrap();
        out.push((dir.to_string(), vandorf::check_files(&p.files).unwrap_or_else(|e| panic!("{dir}: {e:?}"))));
    }
    let f = root.join("tests/fixtures/test1.sys");
    let src = std::fs::read_to_string(&f).unwrap();
    out.push(("test1.sys".into(), vandorf::check(&src).unwrap()));
    out
}

pub fn ty(t: &r::Type) -> v::Type {
    match t {
        r::Type::Prim(p) => v::Type::Prim(format!("{p:?}").to_lowercase().into()),
        r::Type::Array(e) => v::Type::Array(Arc::new(ty(e))),
        r::Type::Map(k, val) => v::Type::Map(Arc::new(v::TypeMap { key: Arc::new(ty(k)), value: Arc::new(ty(val)) })),
        r::Type::Struct(ms) => v::Type::Struct(members(ms)),
        r::Type::Enum(ms) => v::Type::Enum(members(ms)),
        r::Type::Named(id, n) => v::Type::Named(v::TypeNamed { id: *id as i64, name: n.as_str().into() }),
    }
}

fn members(ms: &[(String, r::Type)]) -> v::List<v::Member> {
    ms.iter().map(|(n, t)| v::Member { name: n.as_str().into(), ty: ty(t) }).collect()
}

pub fn defs(ds: &[r::TypeDef]) -> v::List<v::TypeDef> {
    ds.iter()
        .map(|d| v::TypeDef { name: d.name.as_str().into(), ident: d.ident.as_str().into(), ty: ty(&d.ty), recursive: d.recursive })
        .collect()
}

#[test]
fn shows_types_like_the_reference() {
    for (name, prog) in programs() {
        for d in &prog.types {
            let got = vcc::vcc::run_show_type(ty(&d.ty)).unwrap();
            assert_eq!(&*got, d.ty.to_string(), "{name}: {}", d.name);
        }
    }
}

#[test]
fn accepts_like_the_reference() {
    for (name, prog) in programs() {
        let ds = defs(&prog.types);
        let n = prog.types.len();
        for i in 0..n {
            for j in 0..n {
                let (a, b) = (r::Type::Named(i, prog.types[i].name.clone()), r::Type::Named(j, prog.types[j].name.clone()));
                let want = a.accepts(&b, &prog.types);
                let got = vcc::vcc::run_accepts(v::AcceptsIn { a: ty(&a), b: ty(&b), defs: ds.clone(), assumed: Default::default() }).unwrap();
                assert_eq!(got, want, "{name}: {} accepts {}", prog.types[i].name, prog.types[j].name);
            }
        }
    }
}

/// vcc's typedef phase: the typedefs it resolves (a prefix of the
/// reference's, which adds instances used in signatures) and its errors.
fn vcc_types(srcs: &[&str]) -> (Vec<(String, String, String, bool)>, Vec<(String, usize, usize)>) {
    let mut decls = vec![];
    for s in srcs {
        decls.extend(vcc::vcc::run_parse((*s).into()).unwrap_or_else(|e| panic!("{e}")).types.iter().cloned());
    }
    let cx = vcc::vcc::run_types_of(decls.into()).unwrap();
    let types = cx.types.iter().map(|d| (d.name.to_string(), d.ident.to_string(), vcc::vcc::run_show_type(d.ty.clone()).unwrap().to_string(), d.recursive)).collect();
    let errors = cx.errors.iter().map(|e| (e.msg.to_string(), e.span.at as usize, e.span.end as usize)).collect();
    (types, errors)
}

fn ref_types(prog: &vandorf::sema::Program) -> Vec<(String, String, String, bool)> {
    prog.types.iter().map(|d| (d.name.clone(), d.ident.clone(), d.ty.to_string(), d.recursive)).collect()
}

#[test]
fn resolves_typedefs_like_the_reference() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for dir in ["selfhost", "examples/demo"] {
        let p = vandorf::project::load(&root.join(dir).join("vandorf.toml")).unwrap();
        let prog = vandorf::check_files(&p.files).unwrap();
        let srcs: Vec<&str> = p.files.iter().map(|f| f.text.as_str()).collect();
        let (types, errors) = vcc_types(&srcs);
        assert!(errors.is_empty(), "{dir}: {errors:?}");
        let want = ref_types(&prog);
        assert_eq!(types, want[..types.len()], "{dir}");
    }
}

/// Typedef programs, valid and not: the typedef tests of tests/pipeline.rs.
const TYPEDEF_CASES: &[&str] = &[
    "type A = B; type B = A;",
    "type L = []L;",
    "type U = void;",
    "type int = string;",
    "type A = int; type A = string;",
    "type E = enum { A, A };",
    "type S = { a: int, a: int };",
    "type M = map[float]int;",
    "type R<T> = { x: T, pos: int }; type L<T> = enum { Nil, Cons({ h: T, t: L<T> }) }; type X = R<L<[]int>>; type Y = map[string]R<int>;",
    "type R<T> = { x: T }; type A = R;",
    "type R<T> = { x: T }; type A = R<int, int>;",
    "type A = int; type B = A<int>;",
    "type R<T> = { x: int };",
    "type R<T, T> = { x: T };",
    "type R<int> = { x: int };",
    "type R<T> = { x: Q };",
    "type R<T> = { x: R<[]T> }; type A = R<int>;",
    "type W<T> = W<T>; type A = W<int>;",
    "type R<T> = { x: T }; type A = R<{ a: int }>;",
    "type R<T> = { x: T }; type A = R<void>;",
    "type M<K> = { m: map[K]int }; type A = M<float>;",
    "type Id<T> = T; type A = Id<Id<int>>; type B = Id<A>;",
    "type T = enum { Leaf(int), Node({ kids: []T, m: map[string]T }) };",
];

#[test]
fn reports_typedef_errors_like_the_reference() {
    for src in TYPEDEF_CASES {
        let full = format!("{src} proc P(X): int {{ func X(): int; }}");
        let (types, errors) = vcc_types(&[&full]);
        let file = vandorf::parser::parse(&full, 0).unwrap();
        match vandorf::sema::analyze(&file) {
            Ok(prog) => {
                assert!(errors.is_empty(), "{src}: vcc reports {errors:?}");
                assert_eq!(types, ref_types(&prog)[..types.len()], "{src}");
            }
            Err(es) => {
                let want: Vec<(String, usize, usize)> = es.iter().map(|e| (e.msg.clone(), e.span.start, e.span.end)).collect();
                assert_eq!(errors, want, "{src}");
            }
        }
    }
}

// ---- Declarations ----

fn show(t: &v::Type) -> String {
    vcc::vcc::run_show_type(t.clone()).unwrap().to_string()
}

fn ref_members(ms: &[(String, r::Type)]) -> String {
    ms.iter().map(|(n, t)| format!("{n}: {t}")).collect::<Vec<_>>().join(", ")
}

fn vcc_members(ms: &[v::Member]) -> String {
    ms.iter().map(|m| format!("{}: {}", m.name, show(&m.ty))).collect::<Vec<_>>().join(", ")
}

/// Each node as one line, the same for both.
fn ref_nodes(prog: &vandorf::sema::Program) -> Vec<String> {
    use vandorf::sema::NodeKind as K;
    let mut out: Vec<String> = prog
        .nodes
        .iter()
        .map(|n| {
            let kind = match &n.kind {
                K::Func(f) => {
                    let imps: Vec<String> = f.imps.iter().map(|i| format!("{}:{}", i.written, i.function)).collect();
                    format!("func ({}): {} = {}", ref_members(&f.params), f.ret, imps.join(", "))
                }
                K::Proc(p) => {
                    let parent = p.parent.map_or(-1, |x| x as i64);
                    format!("proc parent={parent} entry={} ret={} locals={:?}", p.entry, p.ret, p.locals)
                }
                K::Alias(b) => format!("alias of {b}"),
                K::Id(ps) => format!("id ({})", ref_members(ps)),
            };
            format!("{} @{}..{} {kind}", n.qual.join("."), n.span.start, n.span.end)
        })
        .collect();
    for (name, g) in &prog.globals {
        out.push(match g {
            vandorf::sema::Global::Node(n) => format!("global {name} = {n}"),
            vandorf::sema::Global::Interface(fs) => format!("global {name} = interface {fs:?}"),
        });
    }
    out.push(format!("top {:?}", prog.top_procs));
    out
}

fn vcc_nodes(cx: &v::Cx) -> Vec<String> {
    use v::NodeKind as K;
    let mut out: Vec<String> = cx
        .nodes
        .iter()
        .map(|n| {
            let kind = match &n.kind {
                K::Func(f) => {
                    let imps: Vec<String> = f.imps.iter().map(|i| format!("{}:{}", i.file, i.function)).collect();
                    format!("func ({}): {} = {}", vcc_members(&f.params), show(&f.ret), imps.join(", "))
                }
                K::Proc(p) => {
                    let locals: Vec<usize> = p.locals.iter().map(|&l| l as usize).collect();
                    format!("proc parent={} entry={} ret={} locals={locals:?}", p.parent, p.entry, show(&p.ret))
                }
                K::Alias(b) => format!("alias of {b}"),
                K::Id(ps) => format!("id ({})", vcc_members(ps)),
            };
            let qual: Vec<&str> = n.qual.iter().map(|s| &**s).collect();
            format!("{} @{}..{} {kind}", qual.join("."), n.span.at, n.span.end)
        })
        .collect();
    for (name, g) in cx.globals.iter() {
        out.push(match g {
            v::Global::Node(n) => format!("global {name} = {n}"),
            v::Global::Interface(fs) => {
                let fs: std::collections::BTreeMap<String, usize> = fs.iter().map(|(k, v)| (k.to_string(), *v as usize)).collect();
                format!("global {name} = interface {fs:?}")
            }
        });
    }
    let top: Vec<usize> = cx.top.iter().map(|&t| t as usize).collect();
    out.push(format!("top {top:?}"));
    out
}

/// vcc's declarations of one source, against the reference's.
fn check_decls(src: &str) {
    let file = vandorf::parser::parse(src, 0).unwrap();
    let vfile = vcc::vcc::run_parse(src.into()).unwrap();
    let cx = vcc::vcc::run_declare(vfile).unwrap();
    let errors: Vec<(String, usize, usize)> = cx.ty.errors.iter().map(|e| (e.msg.to_string(), e.span.at as usize, e.span.end as usize)).collect();
    match vandorf::sema::analyze(&file) {
        Ok(prog) => {
            assert!(errors.is_empty(), "{src}: vcc reports {errors:?}");
            let (a, b) = (ref_nodes(&prog), vcc_nodes(&cx));
            assert!(a == b, "{src}\nreference:\n{}\nvcc:\n{}", a.join("\n"), b.join("\n"));
        }
        // Errors vcc doesn't report yet come from building graphs.
        Err(_) if errors.is_empty() => {}
        Err(es) => {
            let want: Vec<(String, usize, usize)> = es.iter().map(|e| (e.msg.clone(), e.span.start, e.span.end)).collect();
            assert_eq!(errors, want, "{src}");
        }
    }
}

const DECL_CASES: &[&str] = &[
    r#"type S = { a: int, b: string, c: { d: int } };
       proc P(A): S {
           func A(): S;
           func N(s: S): int;
           func W(w: { a: int, b: string, c: { d: int }, n: int }): int;
           A -> N(s = { ..this, a = 1 });
           N -> W(w = { ..A, n = { ..A, b = "x" }.a, c = { ..A.c, d = 2 } });
           W -> id({ ..A });
       }"#,
    r#"interface IO { func Read(p: string): string = "io.go:Read", "io.rs:read"; func Read(): int; }
       func Top(x: int): R<int>;
       type R<T> = { x: T };
       proc P(A): int {
           func A(): int;
           proc Q(B): int { func B(): int; B -> A(); };
           A -> Q() as Q2;
           Q2 switch { (this > 0) -> id(1), else -> Q() as Q2 };
       }
       proc Main(S): int { func S(): int; S -> P(); }"#,
    "proc P(A): int { func A(): int; func A(): int; }",
    "proc P(A): int { func B(): int; }",
    "proc P(A): int { func A(): int; A -> id(1) as X; }",
    "proc P(A): int { func A(): int; func B(): int; A -> B() as B; }",
    "proc P(A): int { func A(): int; func B(): int; func C(): int; A -> B() as X; X -> C() as X; }",
    "proc P(A): int { func A(): int; func B(): int; A -> B() as X; X -> X() as Y; }",
    "proc P(A): int { func A(): int; A -> Nope() as X; }",
    "interface I { func F(): int; } proc P(A): int { func A(): int; A -> I() as X; }",
    "interface I { func F(): int; } proc P(A): int { func A(): int; A -> I.G() as X; }",
    "proc P(A): int { func A(): int; A -> A.x() as X; }",
    "func F(): int; func F(): int; proc P(A): int { func A(): int; }",
    "proc P(A): Q { func A(): int; }",
    "proc P(A): int { func A(x: Q): int; }",
];

#[test]
fn declares_like_the_reference() {
    for src in DECL_CASES {
        check_decls(src);
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for f in ["tests/fixtures/test1.sys", "examples/demo/demo.sys", "examples/demo/calc.sys"] {
        if let Ok(src) = std::fs::read_to_string(root.join(f)) {
            check_decls(&src);
        }
    }
}

#[test]
fn declares_projects_like_the_reference() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for dir in ["selfhost", "examples/demo"] {
        let p = vandorf::project::load(&root.join(dir).join("vandorf.toml")).unwrap();
        let prog = vandorf::check_files(&p.files).unwrap();
        let mut all = v::File { types: Default::default(), interfaces: Default::default(), nodes: Default::default() };
        for f in &p.files {
            let vf = vcc::vcc::run_parse(f.text.as_str().into()).unwrap();
            all = v::File { types: all.types.concat(&vf.types), interfaces: all.interfaces.concat(&vf.interfaces), nodes: all.nodes.concat(&vf.nodes) };
        }
        let cx = vcc::vcc::run_declare(all).unwrap();
        assert!(cx.ty.errors.is_empty(), "{dir}: {:?}", cx.ty.errors);
        let (a, b) = (ref_nodes(&prog), vcc_nodes(&cx));
        // Spans are per file: compare without them.
        let strip = |xs: Vec<String>| -> Vec<String> { xs.into_iter().map(|l| l.split(" @").next().unwrap().to_string() + &l.split_once(' ').map_or(String::new(), |(_, r)| r.split_once(' ').map_or(String::new(), |(_, k)| format!(" {k}")))).collect() };
        let (a, b) = (strip(a), strip(b));
        assert!(a == b, "{dir}: first difference at {:?}", a.iter().zip(&b).find(|(x, y)| x != y));
    }
}

// ---- Graphs ----

use vandorf::sema as rs;

fn r_expr(e: &rs::Expr) -> String {
    use rs::Expr as E;
    match e {
        E::Ref(r) => r_ref(r),
        E::Int(i) => format!("{i}"),
        E::Float(f) => format!("{f:?}"),
        E::Str(s) => format!("{s:?}"),
        E::Bool(b) => format!("{b}"),
        E::Variant { tag, payload } => format!("{tag}({})", payload.as_ref().map(|p| r_expr(p)).unwrap_or_default()),
        E::Struct(fs) => format!("{{{}}}", fs.iter().map(|(n, e)| format!("{n}={}", r_expr(e))).collect::<Vec<_>>().join(",")),
        E::Array(es) => format!("[{}]", es.iter().map(r_expr).collect::<Vec<_>>().join(",")),
        E::Unary(op, e) => format!("({}{})", if *op == vandorf::ast::UnOp::Not { "!" } else { "-" }, r_expr(e)),
        E::Binary { op, l, r, operand } => format!("({} {} {} :{})", r_expr(l), op.symbol(), r_expr(r), rs_prim(operand)),
        E::Call(c) => r_call(c),
        E::Map(kvs) => format!("[{}]", kvs.iter().map(|(k, v)| format!("{}:{}", r_expr(k), r_expr(v))).collect::<Vec<_>>().join(",")),
        E::Index { e, key, ty } => format!("{}[{}]@{ty}", r_expr(e), r_expr(key)),
        E::Field { e, ty, name } => format!("{}.{name}@{ty}", r_expr(e)),
        E::In { key, map } => format!("({} in {})", r_expr(key), r_expr(map)),
        E::Query { e, ty, q } => format!("{}.{}@{ty}", r_expr(e), format!("{q:?}").to_lowercase()),
        E::Merge { l, r, ty } => format!("({} ++ {})@{ty}", r_expr(l), r_expr(r)),
        E::Remove { m, keys, ty } => format!("({} -- {})@{ty}", r_expr(m), r_expr(keys)),
    }
}

fn rs_prim(p: &r::Prim) -> String {
    format!("{p:?}").to_lowercase()
}

fn r_ref(r: &rs::Ref) -> String {
    let root = match r.root {
        rs::Root::This => "this".to_string(),
        rs::Root::Node(n) => format!("#{n}"),
        rs::Root::Bind => format!("${}", r.bind.as_deref().unwrap()),
    };
    format!("{root}{}:{}@{}..{}", r.fields.iter().map(|f| format!(".{f}")).collect::<String>(), r.ty, r.span.start, r.span.end)
}

fn r_call(c: &rs::Call) -> String {
    let args: Vec<String> = c.args.iter().map(|a| format!("{}:{}={}", a.name, a.param, r_expr(&a.value))).collect();
    format!("call #{}({}) @{}..{} text={:?}", c.target, args.join(", "), c.span.start, c.span.end, c.text)
}

fn r_pat(p: &rs::Pat) -> String {
    use rs::PatKind as K;
    let k = match &p.kind {
        K::Wild => "_".into(),
        K::Bind(b) => format!("${b}"),
        K::Variant { tag, payload } => format!("{tag}({})", payload.as_ref().map(|q| r_pat(q)).unwrap_or_default()),
        K::Struct(fs) => format!("{{{}}}", fs.iter().map(|(n, q)| format!("{n}={}", r_pat(q))).collect::<Vec<_>>().join(",")),
        K::Or(alts) => alts.iter().map(r_pat).collect::<Vec<_>>().join("|"),
    };
    format!("{k}:{}", p.ty)
}

fn r_arms(arms: &[rs::Arm]) -> String {
    arms.iter()
        .map(|a| {
            let test = match &a.test {
                rs::Test::Pat(p) => r_pat(p),
                rs::Test::Cond(e) => format!("({})", r_expr(e)),
                rs::Test::Else => "else".into(),
            };
            let body = match &a.body {
                rs::Body::Call(c) => r_call(c),
                rs::Body::Switch(inner) => format!("switch {{ {} }}", r_arms(inner)),
            };
            format!("[{}] {test} -> {body}", a.label)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn r_graph(prog: &rs::Program) -> Vec<String> {
    let mut out = vec![];
    for (id, n) in prog.nodes.iter().enumerate() {
        let rs::NodeKind::Proc(p) = &n.kind else { continue };
        let g = &p.graph;
        let mut edges: Vec<_> = g.edges.iter().collect();
        edges.sort_by_key(|(k, _)| **k);
        for (src, e) in edges {
            out.push(format!("#{id} edge #{src}: {}", match e {
                rs::Edge::Call(c) => r_call(c),
                rs::Edge::Match(arms) => format!("switch {}", r_arms(arms)),
                rs::Edge::ForEach { dim, elem, call } => format!("for each {} :{elem} -> {}", r_expr(dim), r_call(call)),
                rs::Edge::Collect(c) => format!("collect -> {}", r_call(c)),
            }));
        }
        for (i, r) in g.regions.iter().enumerate() {
            out.push(format!("#{id} region {i}: parent={} entry={} elem={} nodes={:?} sink={}", r.parent.map_or(-1, |x| x as i64), r.entry, r.elem, r.nodes, r.sink.map_or(-1, |x| x as i64)));
        }
        let ro: std::collections::BTreeMap<_, _> = g.region_of.iter().collect();
        let op: std::collections::BTreeMap<_, _> = g.opens.iter().collect();
        out.push(format!("#{id} region_of={ro:?} opens={op:?}"));
    }
    out
}

fn v_expr(e: &v::Expr) -> String {
    use v::Expr as E;
    match e {
        E::Ref(r) => v_ref(r),
        E::Int(i) => format!("{i}"),
        E::Float(f) => format!("{f:?}"),
        E::Str(s) => format!("{:?}", &**s),
        E::Bool(b) => format!("{b}"),
        E::Variant(x) => format!("{}({})", x.tag, match &*x.payload {
            v::MaybeExpr::Some(p) => v_expr(p),
            v::MaybeExpr::None => String::new(),
        }),
        E::Struct(fs) => format!("{{{}}}", fs.iter().map(|f| format!("{}={}", f.name, v_expr(&f.e))).collect::<Vec<_>>().join(",")),
        E::Array(es) => format!("[{}]", es.iter().map(v_expr).collect::<Vec<_>>().join(",")),
        E::Unary(u) => format!("({}{})", u.op, v_expr(&u.e)),
        E::Binary(b) => format!("({} {} {} :{})", v_expr(&b.l), b.op, v_expr(&b.r), b.operand),
        E::Call(c) => v_call(c),
        E::Map(kvs) => format!("[{}]", kvs.iter().map(|kv| format!("{}:{}", v_expr(&kv.key), v_expr(&kv.value))).collect::<Vec<_>>().join(",")),
        E::Index(x) => format!("{}[{}]@{}", v_expr(&x.e), v_expr(&x.key), show(&x.ty)),
        E::Field(x) => format!("{}.{}@{}", v_expr(&x.e), x.name, show(&x.ty)),
        E::In(x) => format!("({} in {})", v_expr(&x.key), v_expr(&x.map)),
        E::Query(x) => format!("{}.{}@{}", v_expr(&x.e), x.q, show(&x.ty)),
        E::Merge(x) => format!("({} ++ {})@{}", v_expr(&x.l), v_expr(&x.r), show(&x.ty)),
        E::Remove(x) => format!("({} -- {})@{}", v_expr(&x.m), v_expr(&x.keys), show(&x.ty)),
    }
}

fn v_ref(r: &v::Ref) -> String {
    let root = match &r.root {
        v::Root::This => "this".to_string(),
        v::Root::Node(n) => format!("#{n}"),
        v::Root::Bind => format!("${}", r.bind),
    };
    format!("{root}{}:{}@{}..{}", r.fields.iter().map(|f| format!(".{f}")).collect::<String>(), show(&r.ty), r.span.at, r.span.end)
}

fn v_call(c: &v::Call) -> String {
    let args: Vec<String> = c.args.iter().map(|a| format!("{}:{}={}", a.name, show(&a.param), v_expr(&a.value))).collect();
    format!("call #{}({}) @{}..{} text={:?}", c.target, args.join(", "), c.span.at, c.span.end, &*c.text)
}

fn v_pat(p: &v::SPat) -> String {
    use v::SPatKind as K;
    let k = match &*p.kind {
        K::Wild => "_".into(),
        K::Bind(b) => format!("${b}"),
        K::Variant(x) => format!("{}({})", x.tag, match &*x.payload {
            v::MaybePat::Some(q) => v_pat(q),
            v::MaybePat::None => String::new(),
        }),
        K::Struct(fs) => format!("{{{}}}", fs.iter().map(|f| format!("{}={}", f.name, v_pat(&f.pat))).collect::<Vec<_>>().join(",")),
        K::Or(alts) => alts.iter().map(v_pat).collect::<Vec<_>>().join("|"),
    };
    format!("{k}:{}", show(&p.ty))
}

fn v_arms(arms: &[v::SArm]) -> String {
    arms.iter()
        .map(|a| {
            let test = match &a.test {
                v::Test::Pat(p) => v_pat(p),
                v::Test::Cond(e) => format!("({})", v_expr(e)),
                v::Test::Else => "else".into(),
            };
            let body = match &a.body {
                v::SBody::Call(c) => v_call(c),
                v::SBody::Switch(inner) => format!("switch {{ {} }}", v_arms(inner)),
            };
            format!("[{}] {test} -> {body}", a.label)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn v_graph(prog: &v::Program) -> Vec<String> {
    let mut out = vec![];
    for (id, n) in prog.nodes.iter().enumerate() {
        let v::NodeKind::Proc(p) = &n.kind else { continue };
        let g = &p.graph;
        for (src, e) in g.edges.iter() {
            out.push(format!("#{id} edge #{src}: {}", match e {
                v::SEdge::Call(c) => v_call(c),
                v::SEdge::Match(arms) => format!("switch {}", v_arms(arms)),
                v::SEdge::ForEach(f) => format!("for each {} :{} -> {}", v_expr(&f.dim), show(&f.elem), v_call(&f.call)),
                v::SEdge::Collect(c) => format!("collect -> {}", v_call(c)),
            }));
        }
        for (i, r) in g.regions.iter().enumerate() {
            let nodes: Vec<usize> = r.nodes.iter().map(|&x| x as usize).collect();
            out.push(format!("#{id} region {i}: parent={} entry={} elem={} nodes={nodes:?} sink={}", r.parent, r.entry, show(&r.elem), r.sink));
        }
        let ro: std::collections::BTreeMap<usize, usize> = g.region_of.iter().map(|(k, v)| (*k as usize, *v as usize)).collect();
        let op: std::collections::BTreeMap<usize, usize> = g.opens.iter().map(|(k, v)| (*k as usize, *v as usize)).collect();
        out.push(format!("#{id} region_of={:?} opens={:?}", ro.iter().collect::<std::collections::BTreeMap<_, _>>(), op.iter().collect::<std::collections::BTreeMap<_, _>>()));
    }
    out
}

/// A span's end with trailing whitespace and `//` comments trimmed: the
/// reference's spans take in what follows a token.
fn trim_end(src: &str, at: usize, end: usize) -> usize {
    let mut end = end.min(src.len());
    loop {
        let text = &src[at.min(end)..end];
        let trimmed = text.trim_end();
        let cut = match trimmed.rfind('\n') {
            Some(nl) if trimmed[nl..].trim_start().starts_with("//") => nl,
            None if trimmed.trim_start().starts_with("//") && !trimmed.is_empty() => 0,
            _ => trimmed.len(),
        };
        let new = at.min(end) + cut;
        if new == end {
            return end;
        }
        end = new;
    }
}

/// Every `@a..b` in a dump, its end trimmed.
fn trim_spans(src: &str, line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find('@') {
        out.push_str(&rest[..=i]);
        rest = &rest[i + 1..];
        let digits = |s: &str| s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let n = digits(rest);
        if n > 0 && rest[n..].starts_with("..") {
            let m = digits(&rest[n + 2..]);
            let (a, b): (usize, usize) = (rest[..n].parse().unwrap(), rest[n + 2..n + 2 + m].parse().unwrap());
            out.push_str(&format!("{a}..{}", trim_end(src, a, b)));
            rest = &rest[n + 2 + m..];
        }
    }
    out.push_str(rest);
    out
}

/// All of vcc's sema on one source, against the reference's.
fn check_analysis(src: &str) {
    let Ok(file) = vandorf::parser::parse(src, 0) else { return };
    let vfile = vcc::vcc::run_parse(src.into()).unwrap_or_else(|e| panic!("{src}: {e}"));
    let a = vcc::vcc::run_analyze(v::AnalyzeAtS { srcs: vec![v::Str::from(src)].into(), file: vfile }).unwrap_or_else(|e| panic!("{src}: {e}"));
    let errors: Vec<(String, usize, usize)> = a.errors.iter().map(|e| (e.msg.to_string(), e.span.at as usize, trim_end(src, e.span.at as usize, e.span.end as usize))).collect();
    match vandorf::sema::analyze(&file) {
        Ok(prog) => {
            assert!(errors.is_empty(), "{src}\nvcc reports {errors:?}");
            let trim = |xs: Vec<String>| -> Vec<String> { xs.iter().map(|l| trim_spans(src, l)).collect() };
            let (x, y) = (trim(r_graph(&prog)), trim(v_graph(&a.prog)));
            if x != y {
                let first = x.iter().zip(&y).find(|(p, q)| p != q);
                panic!("{src}\nfirst difference:\n  reference: {:?}\n  vcc:       {:?}\n({} vs {} lines)", first.map(|f| f.0), first.map(|f| f.1), x.len(), y.len());
            }
        }
        Err(es) => {
            let want: Vec<(String, usize, usize)> = es.iter().map(|e| (e.msg.clone(), e.span.start, trim_end(src, e.span.start, e.span.end))).collect();
            assert_eq!(errors, want, "{src}");
        }
    }
}

/// The sources compiled in tests/pipeline.rs: string literals passed to
/// compile(..) and compile_err(..), and the SRC/MAIN constants.
fn pipeline_sources() -> Vec<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join("tests/pipeline.rs")).unwrap();
    let mut out = vec![];
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if text[i..].starts_with("r#\"") {
            let end = text[i + 3..].find("\"#").unwrap() + i + 3;
            out.push(text[i + 3..end].to_string());
            i = end + 2;
        } else if b[i] == b'"' {
            let mut s = String::new();
            let mut j = i + 1;
            while b[j] != b'"' {
                if b[j] == b'\\' {
                    j += 1;
                    s.push(match b[j] { b'n' => '\n', b't' => '\t', c => c as char });
                } else {
                    s.push(b[j] as char);
                }
                j += 1;
            }
            out.push(s);
            i = j + 1;
        } else {
            i += 1;
        }
    }
    // Only those that look like programs.
    out.retain(|s| s.contains("proc ") || s.contains("type ") || s.contains("func "));
    out
}

#[test]
fn analyzes_pipeline_programs_like_the_reference() {
    let srcs = pipeline_sources();
    assert!(srcs.len() > 50, "{}", srcs.len());
    let (mut ok, mut errs, mut skipped) = (0, 0, 0);
    for s in &srcs {
        match vandorf::parser::parse(s, 0) {
            Err(_) => skipped += 1,
            Ok(f) => if vandorf::sema::analyze(&f).is_ok() { ok += 1 } else { errs += 1 },
        }
        check_analysis(s);
    }
    eprintln!("pipeline programs: {ok} valid, {errs} with errors, {skipped} not parsed");
    assert!(ok > 20 && errs > 30, "{ok} {errs}");
    for src in TYPEDEF_CASES {
        check_analysis(&format!("{src} proc P(X): int {{ func X(): int; }}"));
    }
    for src in DECL_CASES {
        check_analysis(src);
    }
}

#[test]
fn checks_struct_updates_like_the_reference() {
    let pre = "type S = { a: int, b: string }; func F(x: int): S; proc P(A): int { func A(): S; func N(s: S): int; func M(t: { a: int }): int; func W(w: { a: int, b: string, z: int }): int; func V(v: { a: string, b: string }): int; ";
    for src in [
        "A -> N(s = { ..F(x = 1), a = 1 });",
        "A -> N(s = { ..this.a, a = 1 });",
        "A -> N(s = { ..this, z = 1 });",
        "A -> N(s = { ..this, a = 1, a = 2 });",
        "A -> N(s = { ..this, a = \"x\" });",
        "A -> M(t = { ..this, a = 1 });",
        "A -> M(t = { ..this.a });",
        "A -> W(w = { ..this });",
        "A -> W(w = { ..this, z = 3 });",
        "A -> V(v = { ..this });",
        "A -> N(s = { ..this });",
        "A -> M(t = { a = { ..this, b = \"y\" }.a });",
        "A -> M(t = { a = { ..this, q = 1 }.a });",
    ] {
        check_analysis(&format!("{pre}{src} }}"));
    }
}

#[test]
fn analyzes_test1_like_the_reference() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    check_analysis(&std::fs::read_to_string(root.join("tests/fixtures/test1.sys")).unwrap());
}

/// Every `@a..b` in a dump dropped: spans are per file.
fn no_spans(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find('@') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let n = tail.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(tail.len());
        if n > 0 && tail[..n].contains("..") {
            rest = &tail[n..];
        } else {
            out.push('@');
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn analyzes_projects_like_the_reference() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for dir in ["selfhost", "examples/demo"] {
        let p = vandorf::project::load(&root.join(dir).join("vandorf.toml")).unwrap();
        let prog = vandorf::check_files(&p.files).unwrap();
        let mut all = v::File { types: Default::default(), interfaces: Default::default(), nodes: Default::default() };
        for (i, f) in p.files.iter().enumerate() {
            let vf = vcc::vcc::run_parse_file(f.text.as_str().into(), i as i64).unwrap();
            all = v::File { types: all.types.concat(&vf.types), interfaces: all.interfaces.concat(&vf.interfaces), nodes: all.nodes.concat(&vf.nodes) };
        }
        let srcs: v::List<v::Str> = p.files.iter().map(|f| v::Str::from(f.text.as_str())).collect();
        let a = vcc::vcc::run_analyze(v::AnalyzeAtS { srcs, file: all }).unwrap();
        assert!(a.errors.is_empty(), "{dir}: {:?}", a.errors);
        let strip = |xs: Vec<String>| -> Vec<String> { xs.iter().map(|l| no_spans(l)).collect() };
        let (x, y) = (strip(r_graph(&prog)), strip(v_graph(&a.prog)));
        assert_eq!(x.len(), y.len(), "{dir}");
        if let Some((p, q)) = x.iter().zip(&y).find(|(p, q)| p != q) {
            panic!("{dir}: first difference\n  reference: {p}\n  vcc:       {q}");
        }
    }
}
