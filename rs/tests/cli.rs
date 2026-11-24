//! vcc's command line on small projects: files sharing one namespace,
//! errors naming their file, finding and loading a project, and
//! implementation paths rebased onto the output (once tests/pipeline.rs's
//! of the reference).

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh project directory with these files (paths relative to it).
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vcc-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

/// vcc's exit status and stdout plus stderr, run in dir.
fn vcc(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_vcc")).args(args).arg("-C").arg(dir).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr))
}

const MANIFEST: &str = "[project]\nname = \"t\"\nsources = [\"sys\"]\n";

#[test]
fn files_share_one_namespace() {
    // Types, interfaces and procs in one file are usable from another, in any order.
    let dir = project("ns", &[
        ("vandorf.toml", MANIFEST),
        ("sys/a.sys", "proc A(t: T): int { func S(t: T): int; A -> S(t = this.t); S -> B(i = this); }"),
        ("sys/b.sys", "type T = { x: int };\nproc B(i: int): int { func Q(i: int): int; B -> Q(i = this.i); }"),
    ]);
    let (ok, out) = vcc(&dir, &["check"]);
    assert!(ok && out.contains("ok: 2 files, 2 procs"), "{out}");

    // An error names its file and position.
    std::fs::write(dir.join("sys/a.sys"), "proc A(): int { func S(): int; A -> S(); }").unwrap();
    std::fs::write(dir.join("sys/b.sys"), "\n\nproc A(): int { func S(): int; A -> S(); }").unwrap();
    let (ok, out) = vcc(&dir, &["check"]);
    assert!(!ok && out.contains("duplicate name A") && out.contains("sys/b.sys:3:6"), "{out}");

    // So does a parse error.
    std::fs::write(dir.join("sys/b.sys"), "proc {").unwrap();
    let (ok, out) = vcc(&dir, &["check"]);
    assert!(!ok && out.contains("sys/b.sys:1:"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn parse_errors_are_rendered() {
    let dir = project("render", &[("vandorf.toml", MANIFEST), ("sys/a.sys", "proc P(): int { func A(: int; }")]);
    let (ok, out) = vcc(&dir, &["check"]);
    assert!(!ok, "{out}");
    // The message, where, the line, and a caret under the column.
    let tail = "\n --> sys/a.sys:1:24\n  |\n1 | proc P(): int { func A(: int; }\n  |                        ^";
    assert!(out.contains("found ':'") && out.contains(tail), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A minimal crate the Rust output goes in, so its place in the crate is known.
const CRATE: [(&str, &str); 2] = [("rs/Cargo.toml", "[package]\nname = \"t\"\nversion = \"0.0.0\"\nedition = \"2024\"\n"), ("rs/src/lib.rs", "pub mod app;\n")];

#[test]
fn finds_projects_and_rebases_impl_paths() {
    let manifest = "[project]\nname = \"t\"\nsources = [\"sys\"]\n[rust]\nout = \"rs/src/app\"\n[bin.tool]\nentry = \"A\"\n";
    let dir = project("load", &[
        ("vandorf.toml", manifest),
        ("sys/a.sys", r#"proc A(): int { func S(): int = "impl.rs:s"; A -> S(); S -> B(); }"#),
        ("sys/sub/b.sys", r#"proc B(): int { func Q(): int = "../../rs/q.rs:q"; B -> Q(); }"#),
        // Hidden directories aren't sources.
        (".hidden/c.sys", "not even vandorf"),
        ("sys/.hidden/d.sys", "not either"),
        CRATE[0],
        CRATE[1],
    ]);
    // Found from below the manifest.
    let (ok, out) = vcc(&dir.join("sys/sub"), &["check"]);
    assert!(ok && out.contains("ok: 2 files, 2 procs"), "{out}");

    // Impl paths are relative to their .sys file; generated code reaches them
    // from the output directory, rs/src/app.
    let (ok, out) = vcc(&dir, &["build"]);
    assert!(ok, "{out}");
    let modrs = std::fs::read_to_string(dir.join("rs/src/app/mod.rs")).unwrap();
    assert!(modrs.contains("\"../../../sys/impl.rs\"") && modrs.contains("\"../../q.rs\""), "{modrs}");
    assert!(dir.join("rs/src/bin/tool.rs").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rebases_impl_paths_from_the_project_root() {
    // Sources at the manifest's own directory: the .sys file's directory is "".
    let manifest = "[project]\nname = \"t\"\n[rust]\nout = \"rs/src/app\"\n";
    let dir = project("root", &[
        ("vandorf.toml", manifest),
        ("f.sys", r#"proc A(): int { func S(): int = "rs/src/impls.rs:s"; A -> S(); }"#),
        CRATE[0],
        ("rs/src/lib.rs", "pub mod app;\npub mod impls;\n"),
    ]);
    let (ok, out) = vcc(&dir, &["build"]);
    assert!(ok, "{out}");
    // rs/src/impls.rs is ../impls.rs from the output: the crate's impls module.
    let procs = std::fs::read_to_string(dir.join("rs/src/app/procs.rs")).unwrap();
    assert!(procs.contains("impls::s("), "{}", procs.lines().filter(|l| l.contains("s(")).collect::<Vec<_>>().join("\n"));
    std::fs::remove_dir_all(&dir).unwrap();
}
