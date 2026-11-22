//! vcc's document layout (../sys/doc) against the reference
//! (src/document.rs): the same JSON, byte for byte.

use vcc::vcc::types as v;

fn vcc_doc(srcs: &[&str]) -> String {
    let mut all = v::File { types: Default::default(), interfaces: Default::default(), nodes: Default::default() };
    for (i, s) in srcs.iter().enumerate() {
        let f = vcc::vcc::run_parse_file((*s).into(), i as i64).unwrap();
        all = v::File { types: all.types.concat(&f.types), interfaces: all.interfaces.concat(&f.interfaces), nodes: all.nodes.concat(&f.nodes) };
    }
    let a = vcc::vcc::run_analyze(v::AnalyzeAtS { srcs: srcs.iter().map(|s| v::Str::from(*s)).collect(), file: all }).unwrap();
    assert!(a.errors.is_empty(), "{:?}", a.errors);
    vcc::vcc::run_document(a.prog).unwrap().to_string()
}

fn check(name: &str, prog: &vandorf::sema::Program, srcs: &[&str]) {
    let want = serde_json::to_string_pretty(&vandorf::document::generate(prog)).unwrap();
    let got = vcc_doc(srcs);
    if got != want {
        let line = want.lines().zip(got.lines()).position(|(a, b)| a != b);
        panic!("{name}: documents differ at line {line:?}\nreference:\n{}\nvcc:\n{}", want, got);
    }
}

#[test]
fn lays_out_programs_like_the_reference() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join("tests/pipeline.rs")).unwrap();
    // Raw-string programs in the pipeline tests that check.
    let mut n = 0;
    for part in text.split("r#\"").skip(1) {
        let src = part.split("\"#").next().unwrap();
        let Ok(prog) = vandorf::compile("t.sys", src) else { continue };
        check(src, &prog, &[src]);
        n += 1;
    }
    assert!(n > 15, "{n}");
    let src = std::fs::read_to_string(root.join("tests/fixtures/test1.sys")).unwrap();
    check("test1.sys", &vandorf::compile("t.sys", &src).unwrap(), &[&src]);
}

#[test]
fn lays_out_projects_like_the_reference() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for dir in ["selfhost", "examples/demo"] {
        let p = vandorf::project::load(&root.join(dir).join("vandorf.toml")).unwrap();
        let prog = vandorf::check_files(&p.files).unwrap();
        let srcs: Vec<&str> = p.files.iter().map(|f| f.text.as_str()).collect();
        check(dir, &prog, &srcs);
    }
}
