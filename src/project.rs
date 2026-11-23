//! Projects: a `vandorf.toml` manifest and the `.sys` files it covers.
//!
//! ```toml
//! [project]
//! name = "demo"
//! sources = ["sys"]          # directories searched for .sys files; default ["."]
//!
//! [go]                       # optional: generate Go
//! out = "go/app"             # the code package's directory
//! package = "app"            # default: out's last segment
//! module = "example.com/app" # import path of out; default: from go.mod
//! types_dir = "types"        # relative to out; "." puts types in the code package
//! post = ["go vet ./go/..."] # shell commands run after generating Go
//! test = ["go test ./go/..."] # shell commands `vandorf test` runs
//!
//! [rust]                     # optional: generate Rust
//! out = "rs/src/app"         # a module directory under the crate's src/
//! module_dir = "app"         # out relative to src/; default: from Cargo.toml
//! post = ["cargo build --manifest-path rs/Cargo.toml"]
//! test = ["cargo test --manifest-path rs/Cargo.toml"]
//!
//! [http.api]                 # an HTTP server, per configured backend
//! addr = "127.0.0.1:8080"
//! setup = "Setup"            # runs once before serving; its result is each route's ctx
//! routes = { "POST /api/calc" = "Calc" }   # a route runs a top-level proc
//! raw = { "GET /" = "Page" }                # its proc takes and returns HTTP
//! static = { "/" = "web/index.html" }      # files served as they are
//! go = "go/cmd/api"          # Go main package; default <go.out>/../cmd/<name>
//! rust = "bin/api.rs"        # under the crate's src/; default bin/<name>.rs
//!
//! [bin.demo]                 # an executable per configured backend
//! entry = "Main"             # the top-level proc it runs; default "Main"
//! go = "go/cmd/demo"         # Go main package; default <go.out>/../cmd/<name>
//! rust = "bin/demo.rs"       # Rust file under the crate's src/; default bin/<name>.rs
//! ```
//!
//! `vandorf build` builds the libraries ([go], [rust]) and every bin;
//! `vandorf build <bin>` the libraries and that bin. After generating a
//! language it runs that language's `post` commands. `vandorf test` builds
//! everything, then runs each language's `test` commands. Commands run in
//! order, in the manifest's directory (with `sh -c`, or `cmd /C` on
//! Windows); the first that fails fails the command.
//!
//! Every path is relative to the manifest's directory. The project's files
//! share one namespace. Implementation paths (`= "impl.go:F"`) are relative
//! to the `.sys` file naming them; [`rebase_impls`] turns them into paths
//! relative to a backend's output directory.

use crate::Source;
use crate::sema::Program;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

pub const MANIFEST: &str = "vandorf.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub project: Meta,
    pub go: Option<GoTarget>,
    pub rust: Option<RustTarget>,
    #[serde(default)]
    pub bin: BTreeMap<String, Bin>,
    #[serde(default)]
    pub http: BTreeMap<String, Server>,
}

/// `[http.<name>]`: an HTTP server whose routes run top-level procs.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub addr: String,
    /// A top-level proc without parameters, run once before the server
    /// listens; its result is the context every route and raw proc takes as
    /// its `ctx` parameter, which requests can't set.
    pub setup: Option<String>,
    /// `"POST /api/calc" = "Calc"`: the proc's parameters and result as JSON.
    #[serde(default)]
    pub routes: BTreeMap<String, String>,
    /// `"GET /" = "Page"`: the proc takes the request ({ method, path,
    /// query, headers, body }) and returns the response ({ status, headers,
    /// body }).
    #[serde(default)]
    pub raw: BTreeMap<String, String>,
    /// `"/" = "web/index.html"`: files served as they are, embedded in
    /// the executable. Paths are relative to the manifest.
    #[serde(default, rename = "static")]
    pub statics: BTreeMap<String, String>,
    /// Go main package; default <go.out>/../cmd/<name>.
    pub go: Option<String>,
    /// Rust file under the crate's src/; default bin/<name>.rs.
    pub rust: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bin {
    #[serde(default = "default_entry")]
    pub entry: String,
    pub go: Option<String>,
    pub rust: Option<String>,
}

fn default_entry() -> String {
    crate::codegen::MAIN_PROC.into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub name: String,
    #[serde(default = "default_sources")]
    pub sources: Vec<String>,
}

fn default_sources() -> Vec<String> {
    vec![".".into()]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoTarget {
    pub out: String,
    pub package: Option<String>,
    pub module: Option<String>,
    pub types_dir: Option<String>,
    /// Shell commands run after generating Go.
    #[serde(default)]
    pub post: Vec<String>,
    /// Shell commands `vandorf test` runs.
    #[serde(default)]
    pub test: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustTarget {
    pub out: String,
    pub module_dir: Option<String>,
    /// Shell commands run after generating Rust.
    #[serde(default)]
    pub post: Vec<String>,
    /// Shell commands `vandorf test` runs.
    #[serde(default)]
    pub test: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Project {
    /// The manifest's directory, absolute.
    pub root: PathBuf,
    pub manifest: Manifest,
    /// Paths are relative to `root`, `/`-separated, sorted.
    pub files: Vec<Source>,
}

impl Project {
    /// Absolute directory of each file, by file id.
    pub fn dirs(&self) -> Vec<PathBuf> {
        self.files.iter().map(|f| self.root.join(&f.path).parent().unwrap().to_path_buf()).collect()
    }

    /// Replaces the text of the file at `path` (absolute, or relative to the
    /// root), as for unsaved editor contents. Returns whether it's part of
    /// the project.
    pub fn set_text(&mut self, path: &Path, text: String) -> bool {
        let target = normalize(&absolute(path));
        for f in &mut self.files {
            if normalize(&self.root.join(&f.path)) == target {
                f.text = text;
                return true;
            }
        }
        false
    }
}

/// The nearest `vandorf.toml` in `start` or its ancestors.
pub fn find(start: &Path) -> Option<PathBuf> {
    let mut dir = normalize(&absolute(start));
    loop {
        let m = dir.join(MANIFEST);
        if m.is_file() {
            return Some(m);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Reads a manifest and every `.sys` file under its source directories.
pub fn load(manifest: &Path) -> Result<Project, String> {
    let text = std::fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let parsed: Manifest = toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let root = normalize(&absolute(manifest.parent().unwrap_or(Path::new("."))));
    let mut paths = vec![];
    for s in &parsed.project.sources {
        let dir = root.join(s);
        if !dir.is_dir() {
            return Err(format!("{}: source directory {s} does not exist", manifest.display()));
        }
        walk(&dir, &mut paths);
    }
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err(format!("{}: no .sys files under {}", manifest.display(), parsed.project.sources.join(", ")));
    }
    let mut files = vec![];
    for p in paths {
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let rel = p.strip_prefix(&root).unwrap_or(&p);
        files.push(Source { path: slash(rel), text });
    }
    Ok(Project { root, manifest: parsed, files })
}

/// `.sys` files under `dir`, skipping hidden directories and build output.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                walk(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "sys") {
            out.push(normalize(&p));
        }
    }
}

/// Makes each implementation path relative to `out` instead of the `.sys`
/// file naming it (`dirs[file id]` is that file's directory).
pub fn rebase_impls(prog: &mut Program, dirs: &[PathBuf], out: &Path) {
    let out = normalize(&absolute(out));
    prog.rebase_impls(|imp| slash(&relative(&normalize(&absolute(&dirs[imp.span.context].join(&imp.written))), &out)));
}

pub fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Resolves `.` and `..` without touching the filesystem.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// `target` relative to the directory `from`; both absolute and normalized.
pub fn relative(target: &Path, from: &Path) -> PathBuf {
    let (t, f): (Vec<_>, Vec<_>) = (target.components().collect(), from.components().collect());
    let common = t.iter().zip(&f).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..f.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c);
    }
    out
}

/// A path with `/` separators.
pub fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        let r = |t: &str, f: &str| slash(&relative(Path::new(t), Path::new(f)));
        assert_eq!(r("/p/sys/impl.go", "/p/sys"), "impl.go");
        assert_eq!(r("/p/go/app/impl.go", "/p/go/app"), "impl.go");
        assert_eq!(r("/p/go/effects/foo.go", "/p/go/app"), "../effects/foo.go");
        assert_eq!(r("/p/rs/src/impls.rs", "/p/rs/src/app"), "../impls.rs");
        assert_eq!(slash(&normalize(Path::new("/p/sys/../go/./app"))), "/p/go/app");
    }

    #[test]
    fn manifest_rejects_unknown_fields() {
        assert!(toml::from_str::<Manifest>("[project]\nname = \"x\"\n").is_ok());
        let e = toml::from_str::<Manifest>("[project]\nname = \"x\"\n[go]\nout = \"a\"\nmodul = \"m\"\n").unwrap_err();
        assert!(e.to_string().contains("modul"), "{e}");
    }
}
