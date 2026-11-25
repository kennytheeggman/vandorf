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

/// A project with Go and Rust implementations of the funcs in `SYS`, and no
/// post commands, so `vcc build` only generates and checks.
fn impl_project(name: &str, sys: &str, rs: &str, go: &str) -> PathBuf {
    let manifest = "[project]\nname = \"t\"\nsources = [\"sys\"]\n[rust]\nout = \"rs/src/app\"\n[go]\nout = \"go/app\"\n";
    project(name, &[
        ("vandorf.toml", manifest),
        ("sys/a.sys", sys),
        CRATE[0],
        ("rs/src/lib.rs", "pub mod app;\npub mod impls;\n"),
        ("rs/src/impls.rs", rs),
        ("go/go.mod", "module example.com/t\n\ngo 1.22\n"),
        ("go/app/impl.go", go),
    ])
}

/// Funcs over primitives, a generic typedef's instances (`Res<string>` is
/// `ResString`, `R<[]Tok>` `RTokList`), a struct equal to an instance (the
/// same type, `RInt`), an alias, and docs.
const IMPL_SYS: &str = r#"type Res<T> = enum { Ok(T), Err(string) };
type R<T> = { x: T, pos: int };
type Tok = { text: string };
type Ids = []int;

// Adds one.
func Add(n: int): int = "../go/app/impl.go:Add", "../rs/src/impls.rs:add";
func Read(path: string): Res<string> = "../go/app/impl.go:Read", "../rs/src/impls.rs:read";
func At(r: { x: int, pos: int }): R<int> = "../go/app/impl.go:At", "../rs/src/impls.rs:at";
func Toks(r: R<[]Tok>): int = "../go/app/impl.go:Toks", "../rs/src/impls.rs:toks";
// Takes ids,
// on two lines.
func Use(ids: Ids, n: int) = "../go/app/impl.go:Use", "../rs/src/impls.rs:use_ids";
func Pair(a: int, b: int): int = "../go/app/impl.go:Pair", "../rs/src/impls.rs:pair";

proc Main(n: int): int {
	Main -> Add(n = this.n);
}
"#;

const IMPL_RS: &str = r#"use crate::app::types::*;

/// Adds one.
pub fn add(n: i64) -> Result<i64, String> {
    Ok(n + 1)
}

pub fn read(path: Str) -> Result<ResString, String> {
    Ok(ResString::Ok(path))
}

#[allow(dead_code)]
pub fn at(r: crate::app::types::RInt) -> Result<types::RInt, String> {
    Ok(r)
}

pub fn toks(r: RTokList) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
    Ok(r.pos)
}

/// Takes ids,
/// on two lines.
pub fn use_ids(_ids: List<i64>, _: i64) -> Result<(), String> {
    Ok(())
}

pub fn pair(
    a: i64,
    mut b: i64,
) -> Result<i64, String> {
    b += a;
    Ok(b)
}
"#;

const IMPL_GO: &str = r#"package app

import (
	"context"

	"example.com/t/go/app/types"
)

// Adds one.
func Add(ctx context.Context, n int) (int, error) { return n + 1, nil }

func Read(ctx context.Context, path string) (types.ResString, error) {
	return types.ResString{Tag: "Ok", Ok: path}, nil
}

func At(_ context.Context, r types.RInt) (types.RInt, error) { return r, nil }

func Toks(ctx context.Context, r types.RTokList) (n int, err error) { return r.Pos, nil }

// Takes ids,
// on two lines.
func Use(ctx context.Context, _ types.Ids, n int) error { return nil }

func Pair(ctx context.Context, a, b int) (int, error) { return a + b, nil }
"#;

#[test]
fn checks_implementations_against_declarations() {
    // Matching implementations: written as people do, with paths, aliases
    // and `_` for unused parameters.
    let dir = impl_project("impls-ok", IMPL_SYS, IMPL_RS, IMPL_GO);
    let (ok, out) = vcc(&dir, &["build"]);
    assert!(ok && !out.contains("error") && !out.contains("warning"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();

    // Each mismatch is an error naming the declaration, the implementation
    // and the signature it must have; the build stops before compiling.
    let rs = IMPL_RS
        .replace("pub fn add(n: i64)", "pub fn add(n: f64)")
        .replace("pub fn read(path: Str) -> Result<ResString, String>", "pub fn read(path: Str) -> Result<Str, String>")
        .replace("pub fn at(r: crate::app::types::RInt)", "pub fn at(s: crate::app::types::RInt)")
        .replace("pub fn toks(r: RTokList)", "pub fn toks(r: RTokList, extra: i64)")
        .replace("pub fn use_ids(_ids: List<i64>, _: i64) -> Result<(), String>", "pub fn use_ids(_ids: List<i64>, _: i64) -> i64")
        .replace("pub fn pair(\n    a: i64,", "pub fn pair(\n    a: Str,");
    let go = IMPL_GO
        .replace("func Add(ctx context.Context, n int)", "func Add(ctx context.Context, n float64)")
        .replace("func Read(ctx context.Context, path string) (types.ResString, error)", "func Read(ctx context.Context, path string) (string, error)")
        .replace("func At(_ context.Context, r types.RInt)", "func At(_ context.Context, s types.RInt)")
        .replace("func Toks(ctx context.Context, r types.RTokList)", "func Toks(r types.RTokList)")
        .replace("func Use(ctx context.Context, _ types.Ids, n int) error", "func Use(ctx context.Context, _ types.Ids, n int) (int, error)")
        .replace("func Pair(ctx context.Context, a, b int)", "func Pair(ctx context.Context, a, b string)");
    // Each backend's are its own: Go's stop the build before Rust is
    // generated, so each is tried with the other's implementations right.
    let dir = impl_project("impls-bad-rs", IMPL_SYS, &rs, IMPL_GO);
    let (ok, rs_out) = vcc(&dir, &["build"]);
    assert!(!ok, "{rs_out}");
    std::fs::remove_dir_all(&dir).unwrap();
    let dir = impl_project("impls-bad-go", IMPL_SYS, IMPL_RS, &go);
    let (ok, go_out) = vcc(&dir, &["build"]);
    assert!(!ok, "{go_out}");
    assert!(!go_out.contains("in rs/src/impls.rs"), "{go_out}");
    let out = rs_out + &go_out;
    for want in [
        // Rust.
        "error: Add is implemented by fn add in rs/src/impls.rs, whose signature doesn't match: parameter 1 is `n: f64`, expected `n: i64`",
        "Read is implemented by fn read in rs/src/impls.rs, whose signature doesn't match: it returns `Result<Str, _>`, expected `Result<ResString, _>`",
        "At is implemented by fn at in rs/src/impls.rs, whose signature doesn't match: parameter 1 is `s: RInt`, expected `r: RInt`",
        "Toks is implemented by fn toks in rs/src/impls.rs, whose signature doesn't match: it takes 2 parameters, expected 1",
        "Use is implemented by fn use_ids in rs/src/impls.rs, whose signature doesn't match: it returns `i64`, expected `Result<(), _>`",
        "Pair is implemented by fn pair in rs/src/impls.rs, whose signature doesn't match: parameter 1 is `a: Str`, expected `a: i64`",
        // Go.
        "Add is implemented by func Add in go/app/impl.go, whose signature doesn't match: parameter 1 is `n float64`, expected `n int`",
        "Read is implemented by func Read in go/app/impl.go, whose signature doesn't match: it returns `(string, error)`, expected `(ResString, error)`",
        "At is implemented by func At in go/app/impl.go, whose signature doesn't match: parameter 1 is `s RInt`, expected `r RInt`",
        "Toks is implemented by func Toks in go/app/impl.go, whose signature doesn't match: its first parameter is `r RTokList`, expected `ctx context.Context`",
        "Use is implemented by func Use in go/app/impl.go, whose signature doesn't match: it returns `(int, error)`, expected `error`",
        "Pair is implemented by func Pair in go/app/impl.go, whose signature doesn't match: parameter 1 is `a string`, expected `a int`",
        // Where, and what to write.
        " --> sys/a.sys:7:6",
        "= expected:\n    /// Adds one.\n    pub fn add(n: i64) -> Result<i64, E>",
        "= found: fn add(n: f64) -> Result<i64, String>",
        "= found: func Use(ctx context.Context, _ types.Ids, n int) (int, error)",
        "= expected:\n    // Adds one.\n    func Add(ctx context.Context, n int) (int, error)",
        "= expected: pub fn read(path: crate::app::types::Str) -> Result<crate::app::types::ResString, E>",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
    assert!(!out.contains("post-build"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn checks_implementation_docs() {
    // A doc on one side only, or different docs, is an error.
    let rs = IMPL_RS
        .replace("/// Adds one.\n", "")
        .replace("pub fn read(", "/// Reads a file.\npub fn read(")
        .replace("/// Takes ids,\n/// on two lines.", "/// Takes ids, on one line.");
    let go = IMPL_GO
        .replace("// Adds one.\nfunc Add", "// Adds one!\nfunc Add")
        .replace("// Takes ids,\n// on two lines.\n", "//go:noinline\n");
    let dir = impl_project("impls-docs", IMPL_SYS, &rs, IMPL_GO);
    let (ok, out) = vcc(&dir, &["build"]);
    assert!(!ok, "{out}");
    for want in [
        "error: Add has a doc comment, but fn add in rs/src/impls.rs has none",
        "fn read in rs/src/impls.rs has a doc comment, but Read has none",
        "the doc comments of Use and fn use_ids in rs/src/impls.rs differ",
        "= the declaration's:\n    // Takes ids,\n    // on two lines.",
        "= fn use_ids's:\n    /// Takes ids, on one line.",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
    // Docs are checked apart from signatures: these signatures match.
    assert!(!out.contains("signature"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
    let dir = impl_project("impls-docs-go", IMPL_SYS, IMPL_RS, &go);
    let (ok, out) = vcc(&dir, &["build"]);
    assert!(!ok, "{out}");
    for want in [
        "the doc comments of Add and func Add in go/app/impl.go differ",
        // A directive is no one's doc.
        "Use has a doc comment, but func Use in go/app/impl.go has none",
        "= func Add's:\n    // Adds one!",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_implementations_warn_with_their_doc() {
    // A func whose function doesn't exist yet calls a stub: a warning, and
    // what to write, doc included.
    let rs = IMPL_RS.replace("pub fn add(", "pub fn other(");
    let go = IMPL_GO.replace("func Add(", "func Other(");
    let dir = impl_project("impls-missing", IMPL_SYS, &rs, &go);
    let (ok, out) = vcc(&dir, &["build"]);
    assert!(ok, "{out}");
    for want in [
        "warning: Add is implemented by fn add in rs/src/impls.rs, which does not define it",
        "= expected:\n    /// Adds one.\n    pub fn add(n: i64) -> Result<i64, E>",
        "warning: Add is implemented by func Add in go/app/impl.go, which does not define it",
        "= expected:\n    // Adds one.\n    func Add(ctx context.Context, n int) (int, error)",
    ] {
        assert!(out.contains(want), "missing {want:?} in\n{out}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn builds_and_runs_a_python_only_project() {
    let manifest = r#"
[project]
name = "python-only"
sources = ["sys"]

[python]
out = "py/app"
post = ["python3 -m compileall -q py/app"]
test = ["python3 scripts/tool.py 41"]

[bin.tool]
entry = "Main"
python = "scripts/tool.py"

[http.web]
addr = "127.0.0.1:0"
setup = "Setup"
cli = "Command"
routes = { "GET /sum" = "Sum" }
raw = { "POST /echo" = "Echo" }
static = { "/" = "index.txt" }
"#;
    let source = r#"
type Request = { method: string, path: string, query: map[string]string,
                 headers: map[string]string, body: string };
type Response = { status: int, headers: map[string]string, body: string };
proc Main(n: int): int {
    func Add(n: int): int = "../py/impls.py:add";
    Main -> Add(n = this.n);
}
proc Setup(): int { Setup -> id(7); }
proc Sum(ctx: int, add: int): int { Sum -> id(this.ctx + this.add); }
proc Echo(ctx: int, req: Request): Response {
    Echo -> id({status = 201, headers = ["Content-Type": "text/plain"],
                body = this.req.query["prefix"] + this.req.body});
}
proc Command(args: []string): void {
    func Print(s: string) = "../py/impls.py:echo";
    Command -> Print(s = this.args[0]);
}
proc Concurrent(n: int): int {
    func Branch(n: int): int = "../py/impls.py:branch";
    func Note(n: int) = "../py/impls.py:note";
    Concurrent -> spawn { Branch(n = this.n), Branch(n = this.n) as Other,
                          detach Note(n = this.n) } join id(this.Branch + this.Other);
}
proc Copy(m: map[string]int): int {
    func Mutate(m: map[string]int): int = "../py/impls.py:mutate";
    Copy -> Mutate(m = this.m);
    Mutate -> id(Copy.m["x"] + this);
}
"#;
    let dir = project("python", &[
        ("vandorf.toml", manifest),
        ("sys/main.sys", source),
        ("py/impls.py", r#"
from threading import Barrier, Event

barrier = Barrier(2)
noted = Event()

def add(n):
    return n + 1

def branch(n):
    barrier.wait(timeout=5)
    return n

def note(n):
    noted.set()

def echo(s):
    print(s)

def mutate(m):
    m["x"] += 1
    return m["x"]
"#),
        ("index.txt", "hello static"),
    ]);
    let (ok, out) = vcc(&dir, &["test"]);
    assert!(ok && out.contains("42") && out.contains("test: python3 scripts/tool.py 41"), "{out}");
    let output = Command::new("python3").arg(dir.join("py/web.py")).arg("ping").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ping");

    let checks = r#"
import json
import subprocess
import sys
from urllib.request import Request, urlopen

sys.path.insert(0, "py")
from app import VandorfError, procs
import impls
assert procs.run_concurrent(21) == 42
assert impls.noted.wait(5)
data = {"x": 1}
assert procs.run_copy(data) == 3
assert data == {"x": 1}
try:
    procs.run_command(())
except VandorfError as exc:
    assert "index 0 out of bounds" in str(exc)
else:
    raise AssertionError("expected a VandorfError")

with subprocess.Popen([sys.executable, "py/web.py"], stderr=subprocess.PIPE, text=True) as p:
    try:
        ready = p.stderr.readline().strip()
        assert ready.startswith("listening on "), ready
        base = ready.removeprefix("listening on ")
        with urlopen(base + "/", timeout=5) as response:
            assert response.read() == b"hello static"
        with urlopen(base + "/sum?add=2&ctx=100", timeout=5) as response:
            assert json.load(response) == 9
        with urlopen(Request(base + "/echo?prefix=hello+", data=b"world"), timeout=5) as response:
            assert response.status == 201
            assert response.headers["Content-Type"] == "text/plain"
            assert response.read() == b"hello world"
    finally:
        p.terminate()
"#;
    let output = Command::new("python3").args(["-c", checks]).current_dir(&dir).output().unwrap();
    assert!(output.status.success(), "{output:?}");

    std::fs::write(dir.join("py/impls.py"), "def add(n, extra):\n    return n + extra\n").unwrap();
    let (ok, out) = vcc(&dir, &["build"]);
    assert!(!ok && out.contains("expected def add(n: int) -> int"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}
