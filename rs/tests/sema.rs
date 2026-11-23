//! vcc's sema (../sys/sema): declarations, typedefs, how types show and
//! accept each other, and each program's graph or errors, against their
//! golden files (first recorded from the reference's src/sema.rs and
//! src/types.rs).

mod common;

use vcc::vcc::types as v;

fn show(t: &v::Type) -> String {
    vcc::vcc::run_show_type(t.clone()).unwrap().to_string()
}

fn vcc_members(ms: &[v::Member]) -> String {
    ms.iter().map(|m| format!("{}: {}", m.name, show(&m.ty))).collect::<Vec<_>>().join(", ")
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
                v::SEdge::Spawn(sp) => {
                    let bs: Vec<String> = sp.branches.iter().map(|b| format!("{}{}", if b.detach { "detach " } else { "" }, v_call(&b.call))).collect();
                    format!("spawn {{ {} }} join :{} -> {}", bs.join(", "), show(&sp.joined), v_call(&sp.join))
                }
            }));
        }
        let sp: std::collections::BTreeMap<usize, usize> = g.spawned.iter().map(|(k, v)| (*k as usize, *v as usize)).collect();
        out.push(format!("#{id} spawned={:?}", sp.iter().collect::<std::collections::BTreeMap<_, _>>()));
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

/// vcc_types as golden text.
fn vcc_typedefs(srcs: &[&str]) -> String {
    let (types, errors) = vcc_types(srcs);
    if !errors.is_empty() {
        return errors.iter().map(|(m, a, e)| format!("error: {m} @{a}..{e}\n")).collect();
    }
    types.iter().map(|(n, i, t, r)| format!("{n} ({i}{}) = {t}\n", if *r { ", recursive" } else { "" })).collect()
}

/// vcc's declarations of one source as golden text: its nodes, or its errors.
fn vcc_decls(src: &str) -> String {
    let cx = vcc::vcc::run_declare(vcc::vcc::run_parse(src.into()).unwrap()).unwrap();
    if !cx.ty.errors.is_empty() {
        return cx.ty.errors.iter().map(|e| format!("error: {} @{}..{}\n", e.msg, e.span.at, e.span.end)).collect();
    }
    vcc_nodes(&cx).iter().map(|l| l.clone() + "\n").collect()
}

/// All of vcc's sema on one source, against the reference's.
/// vcc's analysis of one source as golden text: its graph, or its errors
/// (spans trimmed of trailing whitespace and comments).
fn vcc_analysis(src: &str) -> String {
    let vfile = vcc::vcc::run_parse(src.into()).unwrap();
    let a = vcc::vcc::run_analyze(v::AnalyzeAtS { srcs: vec![v::Str::from(src)].into(), file: vfile }).unwrap();
    if !a.errors.is_empty() {
        return a.errors.iter().map(|e| format!("error: {} @{}..{}\n", e.msg, e.span.at, trim_end(src, e.span.at as usize, e.span.end as usize))).collect();
    }
    v_graph(&a.prog).iter().map(|l| trim_spans(src, l) + "\n").collect()
}

/// vcc's typedefs of a program: how each shows, then which accept which.
fn vcc_type_checks(defs: &v::List<v::TypeDef>) -> String {
    let mut out = String::new();
    for d in defs.iter() {
        out += &format!("{} = {}\n", d.name, vcc::vcc::run_show_type(d.ty.clone()).unwrap());
    }
    for (i, a) in defs.iter().enumerate() {
        for (j, b) in defs.iter().enumerate() {
            let ta = v::Type::Named(v::TypeNamed { id: i as i64, name: a.name.clone() });
            let tb = v::Type::Named(v::TypeNamed { id: j as i64, name: b.name.clone() });
            let ok = vcc::vcc::run_accepts(v::AcceptsIn { a: ta, b: tb, defs: defs.clone(), assumed: Default::default() }).unwrap();
            out += &format!("{} accepts {}: {ok}\n", a.name, b.name);
        }
    }
    out
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

/// Spawn edges in a proc with nodes A..E and L, valid and not.
const SPAWN_CASES: &[&str] = &[
    "A -> spawn { B(n = this), B(n = this + 1) as B2, C(n = this), detach L(s = \"x\") } join D(b = this.B + this.B2);",
    "A -> spawn { detach L(s = \"a\"), detach L(s = \"b\") as L2 } join id(A);",
    "A -> spawn { L(s = \"a\") } join D(b = A);",
    "A -> spawn { B(n = 1) } join D(b = this.B); B -> D(b = this);",
    "A -> spawn { B(n = 1) } join D(b = this.B); D -> E(b = B);",
    "A -> spawn { B(n = 1), B(n = 2) } join D(b = this.B);",
    "A -> spawn { B(n = 1) } join D(b = this.B); D -> B(n = this);",
    "A -> spawn { id(1) } join D(b = 1);",
    "A -> spawn { detach B(n = 1) } join D(b = this.B);",
    "A -> spawn { B(n = 1) } join D(b = this.C);",
    "A -> spawn { I.B(n = 1), J.B(n = 2) } join D(b = this.B);",
    "A -> spawn { B(n = 1) } join D(b = this.B); D -> spawn { B(n = 2) } join E(b = this.B);",
    "A -> spawn { B(n = 1) } join D(b = this.B); D -> spawn { B(n = 2) as B3 } join E(b = this.B3 + A);",
];

/// The demo project's sources, or a corpus file's.
fn golden_sources(name: &str) -> Vec<String> {
    if name == "examples-demo" { common::project_sources("examples/demo") } else { vec![common::corpus().into_iter().find(|(n, _)| n == name).unwrap().1] }
}

#[test]
fn shows_and_accepts_types() {
    for name in ["examples-demo", "test1"] {
        let srcs = golden_sources(name);
        let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
        common::check("types", name, &vcc_type_checks(&common::analyze(&srcs).unwrap().types));
    }
}

#[test]
fn resolves_typedefs() {
    let srcs = golden_sources("examples-demo");
    let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
    common::check("typedefs", "examples-demo", &vcc_typedefs(&srcs));
}

#[test]
fn reports_typedef_errors() {
    for (i, src) in TYPEDEF_CASES.iter().enumerate() {
        let full = format!("{src} proc P(X): int {{ func X(): int; }}");
        common::check("typedefs", &format!("c{i:02}"), &format!("{src}\n---\n{}", vcc_typedefs(&[&full])));
    }
}

#[test]
fn declares() {
    for (i, src) in DECL_CASES.iter().enumerate() {
        common::check("decls", &format!("c{i:02}"), &format!("{src}\n---\n{}", vcc_decls(src)));
    }
    let test1 = golden_sources("test1").remove(0);
    common::check("decls", "test1", &format!("{test1}\n---\n{}", vcc_decls(&test1)));
}

#[test]
fn declares_projects() {
    let mut all = v::File { types: Default::default(), interfaces: Default::default(), nodes: Default::default() };
    for src in golden_sources("examples-demo") {
        let vf = vcc::vcc::run_parse(src.as_str().into()).unwrap();
        all = v::File { types: all.types.concat(&vf.types), interfaces: all.interfaces.concat(&vf.interfaces), nodes: all.nodes.concat(&vf.nodes) };
    }
    let cx = vcc::vcc::run_declare(all).unwrap();
    assert!(cx.ty.errors.is_empty(), "{:?}", cx.ty.errors);
    // Spans are per file: compared without them.
    let strip = |xs: Vec<String>| -> Vec<String> { xs.into_iter().map(|l| l.split(" @").next().unwrap().to_string() + &l.split_once(' ').map_or(String::new(), |(_, r)| r.split_once(' ').map_or(String::new(), |(_, k)| format!(" {k}")))).collect() };
    common::check("decls", "examples-demo", &strip(vcc_nodes(&cx)).iter().map(|l| l.clone() + "\n").collect::<String>());
}

/// One source's analysis against its golden file; a source that no longer
/// parses must have none.
fn check_analysis(case: &str, src: &str) {
    if vcc::vcc::run_parse(src.into()).is_err() {
        assert!(!common::has("sema", case), "{case} has a golden analysis but no longer parses");
        return;
    }
    common::check("sema", case, &format!("{src}\n---\n{}", vcc_analysis(src)));
}

#[test]
fn analyzes_programs() {
    let srcs = common::corpus();
    assert!(srcs.len() > 50, "{}", srcs.len());
    for (name, s) in &srcs {
        check_analysis(name, s);
    }
    for (i, src) in TYPEDEF_CASES.iter().enumerate() {
        check_analysis(&format!("typedef-c{i:02}"), &format!("{src} proc P(X): int {{ func X(): int; }}"));
    }
    for (i, src) in DECL_CASES.iter().enumerate() {
        check_analysis(&format!("decl-c{i:02}"), src);
    }
    for (i, edges) in SPAWN_CASES.iter().enumerate() {
        check_analysis(&format!("spawn-c{i:02}"), &format!(
            "interface I {{ func B(n: int): int; }} interface J {{ func B(n: int): int; }}
            proc P(A): int {{ func A(): int; func B(n: int): int; func C(n: int): string; func L(s: string): void; func D(b: int): int; func E(b: int): int; {edges} }}"
        ));
    }
}

#[test]
fn checks_struct_updates() {
    let pre = "type S = { a: int, b: string }; func F(x: int): S; proc P(A): int { func A(): S; func N(s: S): int; func M(t: { a: int }): int; func W(w: { a: int, b: string, z: int }): int; func V(v: { a: string, b: string }): int; ";
    for (i, src) in [
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
    ].iter().enumerate() {
        check_analysis(&format!("update-c{i:02}"), &format!("{pre}{src} }}"));
    }
}

#[test]
fn analyzes_projects() {
    let srcs = golden_sources("examples-demo");
    let mut all = v::File { types: Default::default(), interfaces: Default::default(), nodes: Default::default() };
    for (i, s) in srcs.iter().enumerate() {
        let vf = vcc::vcc::run_parse_file(s.as_str().into(), i as i64).unwrap();
        all = v::File { types: all.types.concat(&vf.types), interfaces: all.interfaces.concat(&vf.interfaces), nodes: all.nodes.concat(&vf.nodes) };
    }
    let a = vcc::vcc::run_analyze(v::AnalyzeAtS { srcs: srcs.iter().map(|s| v::Str::from(s.as_str())).collect(), file: all }).unwrap();
    assert!(a.errors.is_empty(), "{:?}", a.errors);
    common::check("sema", "examples-demo", &v_graph(&a.prog).iter().map(|l| no_spans(l) + "\n").collect::<String>());
}
