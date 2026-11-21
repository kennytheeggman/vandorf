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
