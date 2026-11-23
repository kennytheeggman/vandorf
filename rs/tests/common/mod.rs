//! Golden files: the expected output of each check, in tests/golden, first
//! recorded from the reference implementation. With VCC_BLESS=1 set, a
//! check writes what vcc produces instead of comparing, after a change whose
//! new output has been reviewed.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// The programs in tests/corpus, by name, in name order. They're `.sys.txt`
/// so the IDE and the compilers don't take them for sources (many don't
/// check, on purpose).
pub fn corpus() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(root().join("corpus"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.strip_suffix(".sys.txt")?.to_string();
            Some((name, std::fs::read_to_string(&p).unwrap()))
        })
        .collect();
    out.sort();
    out
}

/// A project's .sys files under the repository root, as `vandorf.toml`'s
/// sources name them, in path order.
pub fn project_sources(dir: &str) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(dir);
    let manifest = std::fs::read_to_string(root.join("vandorf.toml")).unwrap();
    let table: toml::Table = manifest.parse().unwrap();
    let sources: Vec<String> = table["project"]
        .get("sources")
        .and_then(|s| s.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_else(|| vec![".".into()]);
    let mut files = vec![];
    for s in sources {
        walk(&root.join(&s), &root, &mut files);
    }
    files.sort();
    files.into_iter().map(|(_, text)| text).collect()
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if p.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                walk(&p, root, out);
            }
        } else if p.extension().is_some_and(|x| x == "sys") {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push((rel, std::fs::read_to_string(&p).unwrap()));
        }
    }
}

/// Whether a golden file is recorded for the case.
pub fn has(group: &str, case: &str) -> bool {
    root().join("golden").join(group).join(format!("{case}.txt")).exists()
}

/// A program's sources, parsed and checked by vcc: the program, or the
/// diagnostics' messages.
pub fn analyze(srcs: &[&str]) -> Result<vcc::vcc::types::Program, Vec<String>> {
    use vcc::vcc::types as v;
    let mut all = v::File { types: Default::default(), interfaces: Default::default(), nodes: Default::default() };
    for (i, s) in srcs.iter().enumerate() {
        let f = vcc::vcc::run_parse_file((*s).into(), i as i64).map_err(|e| vec![e.to_string()])?;
        all = v::File { types: all.types.concat(&f.types), interfaces: all.interfaces.concat(&f.interfaces), nodes: all.nodes.concat(&f.nodes) };
    }
    let a = vcc::vcc::run_analyze(v::AnalyzeAtS { srcs: srcs.iter().map(|s| v::Str::from(*s)).collect(), file: all }).unwrap();
    if a.errors.is_empty() { Ok(a.prog) } else { Err(a.errors.iter().map(|e| e.msg.to_string()).collect()) }
}

/// Checks `got` against tests/golden/<group>/<case>.txt, or with VCC_BLESS
/// set, writes it there.
pub fn check(group: &str, case: &str, got: &str) {
    let path = root().join("golden").join(group).join(format!("{case}.txt"));
    if std::env::var_os("VCC_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("no golden file {}: run with VCC_BLESS=1 to record it", path.display()));
    if want != got {
        let line = want.lines().zip(got.lines()).position(|(a, b)| a != b).unwrap_or(want.lines().count().min(got.lines().count()));
        panic!(
            "{group}/{case} differs from its golden file at line {}:\n  golden: {:?}\n  vcc:    {:?}\n({} vs {} lines; VCC_BLESS=1 records vcc's output after review)",
            line + 1,
            want.lines().nth(line),
            got.lines().nth(line),
            want.lines().count(),
            got.lines().count(),
        );
    }
}
