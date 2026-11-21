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
