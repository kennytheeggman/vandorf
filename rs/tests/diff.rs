//! The diff document (../src/doc/diff.sys): each pair of programs in
//! tests/corpus/diff, `<case>.old.sys.txt` against `<case>.new.sys.txt`, its
//! summary and document against a golden file; and every corpus program and
//! the demo against itself, which must diff to nothing, with the grids of
//! the visualizer document.

mod common;

use std::path::Path;
use vcc::vcc::types as v;

fn side(label: &str, paths: &[&str], srcs: &[&str]) -> v::DiffSide {
    vcc::vcc::run_check_side(v::CheckSideS {
        label: label.into(),
        paths: paths.iter().map(|p| v::Str::from(*p)).collect(),
        srcs: srcs.iter().map(|s| v::Str::from(*s)).collect(),
    })
    .unwrap()
}

fn output(old: &v::DiffSide, new: &v::DiffSide, json: bool) -> String {
    vcc::vcc::run_diff_output(v::DiffOutputS { old: old.clone(), new: new.clone(), json }).unwrap().to_string()
}

fn field<'a>(j: &'a v::Json, key: &str) -> &'a v::Json {
    match j {
        v::Json::Obj(fs) => &fs.iter().find(|f| &*f.key == key).unwrap_or_else(|| panic!("no {key}")).value,
        _ => panic!("not an object: {j:?}"),
    }
}

/// The pairs in tests/corpus/diff, by case name, in name order.
fn pairs() -> Vec<(String, String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/diff");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.unwrap().file_name().to_str()?.strip_suffix(".old.sys.txt").map(String::from))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let read = |which: &str| std::fs::read_to_string(dir.join(format!("{n}.{which}.sys.txt"))).unwrap();
            (n.clone(), read("old"), read("new"))
        })
        .collect()
}

#[test]
fn diffs_pairs() {
    let pairs = pairs();
    assert!(pairs.len() >= 12, "{}", pairs.len());
    for (name, old, new) in pairs {
        let path = format!("{name}.sys");
        let (o, n) = (side("old", &[&path], &[&old]), side("new", &[&path], &[&new]));
        let got = format!("{}\n\n{}\n", output(&o, &n, false), output(&o, &n, true));
        common::check("diff", &name, &got);
    }
}

/// A program diffs against itself to nothing, and its grids are the
/// visualizer document's: an unchanged proc is drawn as it always is.
fn same(name: &str, srcs: &[&str]) -> bool {
    let Ok(prog) = common::analyze(srcs) else { return false };
    let paths: Vec<String> = (0..srcs.len()).map(|i| format!("{name}-{i}.sys")).collect();
    let paths: Vec<&str> = paths.iter().map(|p| p.as_str()).collect();
    let s = side("same", &paths, srcs);
    let doc = vcc::vcc::run_diff(v::DiffPair { old: s.clone(), new: s }).unwrap();
    for k in ["procs", "types", "interfaces", "funcs"] {
        assert_eq!(field(&doc, k), &v::Json::Obj(Default::default()), "{name}: {k}");
    }
    assert_eq!(field(&doc, "files"), &v::Json::Arr(Default::default()), "{name}");
    let want = vcc::vcc::run_document_json(prog).unwrap();
    assert_eq!(field(&doc, "grids"), field(&want, "grids"), "{name}");
    true
}

#[test]
fn unchanged_programs_diff_to_their_documents() {
    let n = common::corpus().iter().filter(|(name, src)| same(name, &[src])).count();
    assert!(n > 15, "{n}");
    let srcs = common::project_sources("examples/demo");
    let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
    assert!(same("examples-demo", &srcs));
}

#[test]
fn checks_procs_apart() {
    // A proc whose graph doesn't build is set aside, the rest checked: the
    // same program, as Analyze sees it, apart from that.
    let src = "proc A(n: int): int { func F(n: int): int; A -> F(n = this.n); }\nproc B(n: int): int { func F(n: int): int; B -> F(n = \"x\"); }";
    let s = side("t", &["t.sys"], &[src]);
    assert!(s.checks);
    assert_eq!(s.failed.keys().map(|k| k.to_string()).collect::<Vec<_>>(), ["B"]);
    assert!(s.failed["B"].error.contains("t.sys:2:"), "{}", s.failed["B"].error);
    assert!(s.failed["B"].text.starts_with("proc B(n: int): int {") && s.failed["B"].text.ends_with('}'));

    // One whose declaration doesn't check is left out, and declared again
    // without; an error outside every proc fails the whole side.
    let src = "type T = { x: int };\n// B's doc.\nproc B(t: Nope): int { func F(n: int): int; B -> F(n = 1); }\nproc A(t: T): int { func F(n: int): int; A -> F(n = this.t.x); }";
    let s = side("t", &["t.sys"], &[src]);
    assert!(s.checks && s.failed.contains_key("B") && !s.failed.contains_key("A"));
    assert!(s.failed["B"].text.starts_with("// B's doc.\nproc B(t: Nope)"), "{}", s.failed["B"].text);
    let s = side("t", &["t.sys"], &["type T = { x: Nope };\nproc A(): int { func F(): int; A -> F(); }"]);
    assert!(!s.checks && s.error.contains("Nope"), "{}", s.error);
}
