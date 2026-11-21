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
