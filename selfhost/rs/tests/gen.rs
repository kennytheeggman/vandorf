//! vcc's code generators (../sys/gen) against the reference
//! (src/codegen): the same type table and the same generated files.

use vandorf::codegen::types as rt;
use vcc::vcc::types as v;

pub fn analyze(srcs: &[&str]) -> v::Program {
    let mut all = v::File { types: Default::default(), interfaces: Default::default(), nodes: Default::default() };
    for (i, s) in srcs.iter().enumerate() {
        let f = vcc::vcc::run_parse_file((*s).into(), i as i64).unwrap();
        all = v::File { types: all.types.concat(&f.types), interfaces: all.interfaces.concat(&f.interfaces), nodes: all.nodes.concat(&f.nodes) };
    }
    let a = vcc::vcc::run_analyze(v::AnalyzeAtS { srcs: srcs.iter().map(|s| v::Str::from(*s)).collect(), file: all }).unwrap();
    assert!(a.errors.is_empty(), "{:?}", a.errors);
    a.prog
}

/// Programs to generate from: (name, reference program, sources).
pub fn programs() -> Vec<(String, vandorf::sema::Program, Vec<String>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = vec![];
    let text = std::fs::read_to_string(root.join("tests/pipeline.rs")).unwrap();
    for part in text.split("r#\"").skip(1) {
        let src = part.split("\"#").next().unwrap();
        if let Ok(prog) = vandorf::compile("t.sys", src) {
            out.push((src.to_string(), prog, vec![src.to_string()]));
        }
    }
    let src = std::fs::read_to_string(root.join("tests/fixtures/test1.sys")).unwrap();
    out.push(("test1.sys".into(), vandorf::compile("t.sys", &src).unwrap(), vec![src]));
    for dir in ["selfhost", "examples/demo"] {
        let p = vandorf::project::load(&root.join(dir).join("vandorf.toml")).unwrap();
        let prog = vandorf::check_files(&p.files).unwrap();
        out.push((dir.to_string(), prog, p.files.iter().map(|f| f.text.clone()).collect()));
    }
    out
}

fn r_tyref(r: &rt::TyRef) -> String {
    match r {
        rt::TyRef::Prim(p) => format!("{p:?}").to_lowercase(),
        rt::TyRef::Array(e) => format!("[]{}", r_tyref(e)),
        rt::TyRef::Map(k, val) => format!("map[{}]{}", r_tyref(k), r_tyref(val)),
        rt::TyRef::Named(i) => format!("#{i}"),
    }
}

fn v_tyref(r: &v::TyRef) -> String {
    match r {
        v::TyRef::Prim(p) => p.to_string(),
        v::TyRef::Array(e) => format!("[]{}", v_tyref(e)),
        v::TyRef::Map(m) => format!("map[{}]{}", v_tyref(&m.key), v_tyref(&m.value)),
        v::TyRef::Named(i) => format!("#{i}"),
    }
}

fn r_table(t: &rt::TypeTable) -> Vec<String> {
    let mut out = vec![];
    for (i, n) in t.types.iter().enumerate() {
        let (kind, ms) = match &n.shape {
            rt::Shape::Struct(ms) => ("struct", ms),
            rt::Shape::Enum(ms) => ("enum", ms),
        };
        let ms: Vec<String> = ms.iter().map(|(f, r)| format!("{f}{}: {}", if t.is_boxed(i, f) { "*" } else { "" }, r_tyref(r))).collect();
        out.push(format!("#{i} {} {kind} {{ {} }} ({})", n.name, ms.join(", "), n.origin));
    }
    for (n, r) in &t.aliases {
        out.push(format!("alias {n} = {}", r_tyref(r)));
    }
    out
}

fn v_table(t: &v::Table) -> Vec<String> {
    let mut out = vec![];
    for (i, n) in t.types.iter().enumerate() {
        let (kind, ms) = match &n.shape {
            v::Shape::Struct(ms) => ("struct", ms),
            v::Shape::Enum(ms) => ("enum", ms),
        };
        let ms: Vec<String> = ms.iter().map(|m| format!("{}{}: {}", m.name, if t.boxed.contains_key(&v::Str::from(format!("{i}:{}", m.name))) { "*" } else { "" }, v_tyref(&m.r))).collect();
        out.push(format!("#{i} {} {kind} {{ {} }} ({})", n.name, ms.join(", "), n.origin));
    }
    for a in t.aliases.iter() {
        out.push(format!("alias {} = {}", a.name, v_tyref(&a.r)));
    }
    out
}

#[test]
fn builds_type_tables_like_the_reference() {
    let progs = programs();
    assert!(progs.len() > 15);
    for (name, prog, srcs) in &progs {
        let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
        let want = r_table(&rt::TypeTable::build(prog));
        let got = v_table(&vcc::vcc::run_build_table(analyze(&srcs)).unwrap());
        if want != got {
            let i = want.iter().zip(&got).position(|(a, b)| a != b);
            panic!("{name}\nfirst difference at {i:?}:\n  reference: {:?}\n  vcc:       {:?}\n({} vs {} lines)", i.map(|i| &want[i]), i.map(|i| &got[i]), want.len(), got.len());
        }
    }
}

#[test]
fn generates_rust_types_like_the_reference() {
    use vandorf::codegen::rust;
    for (name, prog, srcs) in &programs() {
        let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
        let Ok(out) = rust::generate(prog, &rust::RustOptions { module_dir: Some("app".into()), ..Default::default() }) else { continue };
        let want = &out.types.files["types.rs"];
        let table = vcc::vcc::run_build_table(analyze(&srcs)).unwrap();
        let got = vcc::vcc::run_rust_typegen(table).unwrap().to_string();
        if *want != got {
            let line = want.lines().zip(got.lines()).position(|(a, b)| a != b);
            panic!("{name}: types.rs differs at line {line:?}: {:?} vs {:?}", line.map(|l| want.lines().nth(l)), line.map(|l| got.lines().nth(l)));
        }
    }
}

#[test]
fn generates_rust_procs_like_the_reference() {
    use vandorf::codegen::rust;
    let mut n = 0;
    for (name, prog, srcs) in &programs() {
        let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
        let want = rust::generate(prog, &rust::RustOptions { module_dir: Some("app".into()), ..Default::default() });
        let got = vcc::vcc::run_rust_generate(v::RustGenerateAtS { prog: analyze(&srcs), module_dir: v::MaybeStr::Some("app".into()) }).unwrap();
        match (want, &got.err) {
            (Ok(out), v::MaybeStr::None) => {
                for (file, text) in [("procs.rs", &got.procs), ("mod.rs", &got.modrs), ("types.rs", &got.types)] {
                    let want = out.code.files.get(file).or(out.types.files.get(file)).unwrap();
                    if **want != **text {
                        let line = want.lines().zip(text.lines()).position(|(a, b)| a != b);
                        panic!("{name}: {file} differs at line {line:?}:\n  reference: {:?}\n  vcc:       {:?}", line.and_then(|l| want.lines().nth(l)), line.and_then(|l| text.lines().nth(l)));
                    }
                }
                n += 1;
            }
            (Err(e), v::MaybeStr::Some(g)) => assert_eq!(e, g.to_string(), "{name}"),
            (w, g) => panic!("{name}: reference {:?}, vcc {g:?}", w.map(|_| ())),
        }
    }
    assert!(n > 15, "{n}");
}
