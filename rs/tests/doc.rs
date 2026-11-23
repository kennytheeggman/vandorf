//! vcc's document layout (../sys/doc): each valid corpus program's and the
//! demo project's document, against its golden file.

mod common;

/// The document, or None if the sources don't check (then no golden file
/// may be recorded: a program that checked before must still).
fn check(name: &str, srcs: &[&str]) -> bool {
    match common::analyze(srcs) {
        Ok(prog) => {
            common::check("doc", name, &vcc::vcc::run_document(prog).unwrap().to_string());
            true
        }
        Err(e) => {
            assert!(!common::has("doc", name), "{name} has a golden document but no longer checks: {e:?}");
            false
        }
    }
}

#[test]
fn lays_out_programs() {
    let n = common::corpus().iter().filter(|(name, src)| check(name, &[src])).count();
    assert!(n > 15, "{n}");
}

#[test]
fn lays_out_projects() {
    let srcs = common::project_sources("examples/demo");
    let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
    assert!(check("examples-demo", &srcs));
}
