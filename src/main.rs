mod serve;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use vandorf::codegen::go::GoOptions;
use vandorf::codegen::rust::RustOptions;
use vandorf::codegen::{self, MainSpec, Output};
use vandorf::project::{self, Project};
use vandorf::sema::Program;
use vandorf::Source;

const USAGE: &str = "\
usage:
  vandorf build [<bin>] [--manifest vandorf.toml]     # libraries, and every bin or just <bin>
  vandorf check [<file.sys> | --manifest vandorf.toml]
  vandorf doc   [<file.sys> | --manifest vandorf.toml] [-o out.json]
  vandorf go    <file.sys> -o <dir> [--package name] [--module import/path/of/dir] [--types-dir types] [--main] [--entry Proc]
  vandorf rust  <file.sys> -o <dir> [--module-dir path/under/src] [--main] [--entry Proc]
  vandorf serve [--addr 127.0.0.1:8080] [--dir .] [--term | --no-term]   # IDE; terminals default on for loopback only

Without a file, check and doc use the project: the nearest vandorf.toml in the
current directory or its parents (see the README for its fields). A project's
.sys files share one namespace.

Each backend writes two artifacts: the types (typegen) and the procs (codegen).
Impl paths in `= \"file:function\"` are relative to the .sys file naming them;
each backend uses the reference with its extension (.go / .rs).

go:   <dir> is the code package; types go to <dir>/<types-dir> as their own
      package (`.` keeps them in the code package). Import paths come from the
      enclosing go.mod, or --module (the import path of <dir>).
rust: <dir> is a module (<dir>/mod.rs, types.rs, procs.rs). Its place in the
      crate comes from the enclosing Cargo.toml, or --module-dir (relative to
      src/).

--main also writes an executable that runs proc Main (or --entry's proc, which
must exist), taking its parameters as command-line arguments:
<dir>/main/main_vandorf.go for Go, src/main.rs for Rust (which calls the procs
through the crate's lib.rs, so the module must be reachable from there). In a
project, [bin.<name>] tables configure executables instead.";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
struct Args {
    cmd: String,
    input: Option<PathBuf>,
    manifest: Option<PathBuf>,
    out: Option<PathBuf>,
    package: Option<String>,
    module: Option<String>,
    types_dir: Option<String>,
    module_dir: Option<String>,
    main: bool,
    entry: Option<String>,
}

fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut it = argv.into_iter();
    let mut args = Args { cmd: it.next().ok_or(USAGE)?, ..Default::default() };
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{a} needs a value\n\n{USAGE}"));
        match a.as_str() {
            "-o" | "--out" => args.out = Some(PathBuf::from(val()?)),
            "--manifest" => args.manifest = Some(PathBuf::from(val()?)),
            "--package" => args.package = Some(val()?),
            "--module" => args.module = Some(val()?),
            "--types-dir" => args.types_dir = Some(val()?),
            "--module-dir" => args.module_dir = Some(val()?),
            "--main" => args.main = true,
            "--entry" => args.entry = Some(val()?),
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with('-') => return Err(format!("unknown flag {s}\n\n{USAGE}")),
            _ if args.input.is_none() => args.input = Some(PathBuf::from(&a)),
            _ => return Err(format!("unexpected argument {a}\n\n{USAGE}")),
        }
    }
    Ok(args)
}

/// What a command compiles: one file, or a project.
struct Unit {
    files: Vec<Source>,
    /// Absolute directory of each file, for impl paths.
    dirs: Vec<PathBuf>,
    project: Option<Project>,
}

impl Unit {
    fn file(path: &Path) -> Result<Unit, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let dir = project::normalize(&project::absolute(path.parent().unwrap_or(Path::new("."))));
        Ok(Unit { files: vec![Source { path: path.display().to_string(), text }], dirs: vec![dir], project: None })
    }

    fn project(manifest: Option<&Path>) -> Result<Unit, String> {
        let manifest = match manifest {
            Some(m) => m.to_path_buf(),
            None => {
                let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
                project::find(&cwd).ok_or_else(|| format!("no {} in {} or its parents\n\n{USAGE}", project::MANIFEST, cwd.display()))?
            }
        };
        let p = project::load(&manifest)?;
        Ok(Unit { files: p.files.clone(), dirs: p.dirs(), project: Some(p) })
    }

    fn compile(&self) -> Result<Program, String> {
        vandorf::compile_files(&self.files)
    }
}

fn run(argv: Vec<String>) -> Result<(), String> {
    if argv.first().map(String::as_str) == Some("serve") {
        return serve_cmd(&argv[1..]);
    }
    let args = parse_args(argv)?;
    let unit = match (&args.input, args.cmd.as_str()) {
        (_, "build") => Unit::project(args.manifest.as_deref())?,
        (Some(f), _) => Unit::file(f)?,
        (None, "check" | "doc") => Unit::project(args.manifest.as_deref())?,
        (None, _) => return Err(USAGE.into()),
    };
    let mut prog = unit.compile()?;

    match args.cmd.as_str() {
        "check" => {
            eprintln!("ok: {} files, {} procs", unit.files.len(), prog.all_procs().len());
            Ok(())
        }
        "doc" => {
            let json = serde_json::to_string_pretty(&vandorf::document::generate(&prog)).unwrap();
            match &args.out {
                Some(p) => std::fs::write(p, json + "\n").map_err(|e| format!("{}: {e}", p.display())),
                None => {
                    println!("{json}");
                    Ok(())
                }
            }
        }
        "build" => {
            let p = unit.project.as_ref().unwrap();
            let m = &p.manifest;
            if m.go.is_none() && m.rust.is_none() {
                return Err(format!("{}: configure [go] and/or [rust] to build", project::MANIFEST));
            }
            let bins: Vec<(&String, &project::Bin)> = match &args.input {
                None => m.bin.iter().collect(),
                Some(name) => {
                    let name = name.display().to_string();
                    match m.bin.get_key_value(&name) {
                        Some(b) => vec![b],
                        None => {
                            let known: Vec<&str> = m.bin.keys().map(String::as_str).collect();
                            return Err(format!("{}: no [bin.{name}]; bins: {}", project::MANIFEST, if known.is_empty() { "none".into() } else { known.join(", ") }));
                        }
                    }
                }
            };
            if !m.http.is_empty() && m.rust.is_none() {
                return Err(format!("{}: [http.*] servers are generated in Rust: configure [rust]", project::MANIFEST));
            }
            if !m.http.is_empty() && m.go.is_some() {
                eprintln!("note: [http.*] servers are generated in Rust only, for now");
            }
            if let Some(g) = &m.go {
                let out = p.root.join(&g.out);
                go(&mut prog, &unit.dirs, &out, g.package.clone(), g.module.clone(), g.types_dir.clone(), go_mains(p, g, &bins))?;
            }
            if let Some(r) = &m.rust {
                let servers: Vec<(&String, &project::Server)> = if args.input.is_none() { m.http.iter().collect() } else { vec![] };
                rust(&mut prog, &unit.dirs, &p.root.join(&r.out), r.module_dir.clone(), rust_mains(&bins), &servers, &p.root)?;
            }
            Ok(())
        }
        "go" => {
            let out = out_dir(&args)?;
            let mains = single_main(&args, "main");
            go(&mut prog, &unit.dirs, &out, args.package.clone(), args.module.clone(), args.types_dir.clone(), mains)
        }
        "rust" => {
            let out = out_dir(&args)?;
            let mains = single_main(&args, "main.rs");
            rust(&mut prog, &unit.dirs, &out, args.module_dir.clone(), mains, &[], Path::new("."))
        }
        other => Err(format!("unknown command {other}\n\n{USAGE}")),
    }
}

fn go(
    prog: &mut Program,
    dirs: &[PathBuf],
    out: &Path,
    package: Option<String>,
    module: Option<String>,
    types_dir: Option<String>,
    mains: Vec<MainSpec>,
) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    project::rebase_impls(prog, dirs, out);
    let mut opts = GoOptions::new(package.unwrap_or_else(|| default_package(out)), module.or_else(|| go_import_path(out)));
    if let Some(d) = types_dir {
        opts.types_dir = d;
    }
    opts.mains = mains;
    let output = codegen::go::generate(prog, &opts)?;
    let written = write_output(out, &output)?;
    // Generated code is already indented; gofmt only aligns it.
    format_with("gofmt", &["-w"], &written);
    check_impls(out, &output, "func");
    Ok(())
}

/// The Go mains of `bins`: each a package in its `go` directory, by default
/// `<go.out>/../cmd/<name>`, given relative to `go.out`.
fn go_mains(p: &Project, g: &project::GoTarget, bins: &[(&String, &project::Bin)]) -> Vec<MainSpec> {
    let out = project::normalize(&p.root.join(&g.out));
    bins.iter()
        .map(|(name, b)| {
            let dir = b.go.clone().unwrap_or_else(|| format!("{}/../cmd/{name}", g.out));
            let path = project::slash(&project::relative(&project::normalize(&p.root.join(dir)), &out));
            MainSpec { name: name.to_string(), entry: b.entry.clone(), path }
        })
        .collect()
}

/// The Rust mains of `bins`: each a file under the crate's `src/`, by
/// default `bin/<name>.rs` (a Cargo bin target).
fn rust_mains(bins: &[(&String, &project::Bin)]) -> Vec<MainSpec> {
    bins.iter()
        .map(|(name, b)| MainSpec {
            name: name.to_string(),
            entry: b.entry.clone(),
            path: b.rust.clone().unwrap_or_else(|| format!("bin/{name}.rs")),
        })
        .collect()
}

/// `--main` / `--entry` on the single-file commands.
fn single_main(args: &Args, path: &str) -> Vec<MainSpec> {
    if !args.main && args.entry.is_none() {
        return vec![];
    }
    let entry = args.entry.clone().unwrap_or_else(|| codegen::MAIN_PROC.into());
    vec![MainSpec { name: "main".into(), entry, path: path.into() }]
}

fn rust(
    prog: &mut Program,
    dirs: &[PathBuf],
    out: &Path,
    module_dir: Option<String>,
    mains: Vec<MainSpec>,
    servers: &[(&String, &project::Server)],
    root: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    project::rebase_impls(prog, dirs, out);
    let module_dir = module_dir.or_else(|| rust_module_dir(out));
    let servers = servers.iter().map(|(name, s)| server_spec(name, s, root, out, module_dir.as_deref())).collect::<Result<_, _>>()?;
    let opts = RustOptions { module_dir, mains, crate_name: rust_lib_name(out), servers };
    let output = codegen::rust::generate(prog, &opts)?;
    let written = write_output(out, &output)?;
    format_with("rustfmt", &["--edition", "2024"], &written);
    check_impls(out, &output, "fn");
    Ok(())
}

/// An `[http.<name>]` table as the Rust backend takes it: routes split into
/// method and path, static files as the server's source file reaches them.
fn server_spec(name: &str, s: &project::Server, root: &Path, out: &Path, module_dir: Option<&str>) -> Result<codegen::rust::ServerSpec, String> {
    let split = |route: &str| -> Result<(String, String), String> {
        route
            .split_once(' ')
            .map(|(m, p)| (m.trim(), p.trim()))
            .filter(|(m, p)| !m.is_empty() && m.chars().all(|c| c.is_ascii_uppercase()) && p.starts_with('/'))
            .map(|(m, p)| (m.to_string(), p.to_string()))
            .ok_or_else(|| format!("{}: [http.{name}] route {route:?} should be a method and a path, like \"POST /api/calc\"", project::MANIFEST))
    };
    let mut raw = vec![];
    for (route, proc) in &s.raw {
        if s.routes.contains_key(route) {
            return Err(format!("{}: [http.{name}] route {route:?} is in both routes and raw", project::MANIFEST));
        }
        let (m, p) = split(route)?;
        raw.push((m, p, proc.clone()));
    }
    let mut routes = vec![];
    for (route, proc) in &s.routes {
        let Some((method, path)) = route.split_once(' ').map(|(m, p)| (m.trim(), p.trim())).filter(|(m, p)| {
            !m.is_empty() && m.chars().all(|c| c.is_ascii_uppercase()) && p.starts_with('/')
        }) else {
            return Err(format!("{}: [http.{name}] route {route:?} should be a method and a path, like \"POST /api/calc\"", project::MANIFEST));
        };
        routes.push((method.to_string(), path.to_string(), proc.clone()));
    }
    let path = s.rust.clone().unwrap_or_else(|| format!("bin/{name}.rs"));
    // The crate's src/ is module_dir above out.
    let up = module_dir.map_or(0, |d| d.split('/').filter(|x| !x.is_empty()).count());
    let mut src = project::normalize(&project::absolute(out));
    for _ in 0..up {
        src.pop();
    }
    let file_dir = project::normalize(&src.join(&path)).parent().map(Path::to_path_buf).unwrap_or(src);
    let mut statics = vec![];
    for (url, file) in &s.statics {
        let full = project::normalize(&project::absolute(&root.join(file)));
        if !full.is_file() {
            return Err(format!("{}: [http.{name}] serves {file}, which does not exist", project::MANIFEST));
        }
        let ctype = match full.extension().and_then(|e| e.to_str()).unwrap_or("") {
            "html" | "htm" => "text/html; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" => "application/json",
            "svg" => "image/svg+xml",
            _ => "text/plain; charset=utf-8",
        };
        statics.push((url.clone(), project::slash(&project::relative(&full, &file_dir)), ctype.to_string()));
    }
    Ok(codegen::rust::ServerSpec { name: name.into(), addr: s.addr.clone(), routes, raw, statics, path })
}

fn out_dir(args: &Args) -> Result<PathBuf, String> {
    args.out.clone().ok_or_else(|| format!("{} needs -o <dir>\n\n{USAGE}", args.cmd))
}

/// Writes the artifacts, returning the paths written. Refuses to overwrite
/// files vandorf didn't generate (say, a hand-written `src/main.rs`).
fn write_output(out: &Path, output: &Output) -> Result<Vec<PathBuf>, String> {
    let mut arts = vec![("types", &output.types), ("code", &output.code)];
    arts.extend(output.mains.iter().map(|m| ("main", m)));
    for (_, art) in &arts {
        for name in art.files.keys() {
            let p = out.join(&art.dir).join(name);
            if let Ok(old) = std::fs::read_to_string(&p)
                && !old.starts_with("// Code generated by vandorf.")
            {
                return Err(format!("{} exists and was not generated by vandorf; not overwriting it", p.display()));
            }
        }
    }
    let mut written = vec![];
    for (kind, art) in arts {
        let dir = out.join(&art.dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for (name, src) in &art.files {
            let p = dir.join(name);
            std::fs::write(&p, src).map_err(|e| format!("{}: {e}", p.display()))?;
            eprintln!("wrote {:5} {}", kind, p.display());
            written.push(p);
        }
    }
    Ok(written)
}

/// Runs a formatter over the written files if it's installed.
fn format_with(tool: &str, flags: &[&str], files: &[PathBuf]) {
    match std::process::Command::new(tool).args(flags).args(files).status() {
        Ok(s) if !s.success() => eprintln!("note: {tool} failed ({s})"),
        Ok(_) => {}
        Err(e) => eprintln!("note: {tool} not run ({e})"),
    }
}

fn serve_cmd(argv: &[String]) -> Result<(), String> {
    let mut opts = serve::Options { addr: "127.0.0.1:8080".into(), dir: PathBuf::from("."), term: None };
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--term" => opts.term = Some(true),
            "--no-term" => opts.term = Some(false),
            "--addr" | "--dir" => {
                let val = it.next().ok_or_else(|| format!("{a} needs a value\n\n{USAGE}"))?;
                if a == "--addr" { opts.addr = val.clone() } else { opts.dir = PathBuf::from(val) }
            }
            _ => return Err(format!("unknown argument {a}\n\n{USAGE}")),
        }
    }
    serve::serve(opts)
}

fn default_package(dir: &Path) -> String {
    let base = std::fs::canonicalize(dir).ok()
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "generated".into());
    let s: String = base.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if s.starts_with(|c: char| c.is_ascii_digit()) { format!("p{s}") } else { s }
}

/// Finds the nearest ancestor of `dir` containing `marker`, returning it and
/// `dir` relative to it as `/`-separated segments.
fn find_root(dir: &Path, marker: &str) -> Option<(PathBuf, Vec<String>)> {
    let dir = std::fs::canonicalize(dir).ok()?;
    let mut root = dir.as_path();
    loop {
        if root.join(marker).is_file() {
            let rel = dir.strip_prefix(root).ok()?;
            let segs = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            return Some((root.to_path_buf(), segs));
        }
        root = root.parent()?;
    }
}

/// Import path of `dir`, from the nearest enclosing go.mod.
fn go_import_path(dir: &Path) -> Option<String> {
    let (root, rel) = find_root(dir, "go.mod")?;
    let gomod = std::fs::read_to_string(root.join("go.mod")).ok()?;
    let module = gomod.lines().find_map(|l| l.trim().strip_prefix("module "))?.trim().trim_matches('"');
    Some(std::iter::once(module.to_string()).chain(rel).collect::<Vec<_>>().join("/"))
}

/// `dir` relative to the enclosing crate's `src/`.
fn rust_module_dir(dir: &Path) -> Option<String> {
    let (_, rel) = find_root(dir, "Cargo.toml")?;
    match rel.split_first() {
        Some((src, rest)) if src == "src" => Some(rest.join("/")),
        _ => None,
    }
}

/// Name (as a path) of the library crate enclosing `dir`, if it has one.
fn rust_lib_name(dir: &Path) -> Option<String> {
    let (root, _) = find_root(dir, "Cargo.toml")?;
    let toml = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    // Enough TOML for a PoC: `name = "..."` in [lib], else in [package].
    let mut section = "";
    let (mut lib, mut package) = (None, None);
    for l in toml.lines().map(str::trim) {
        if l.starts_with('[') {
            section = l;
        } else if let Some(v) = l.strip_prefix("name").map(str::trim_start).and_then(|v| v.strip_prefix('=')) {
            let v = Some(v.trim().trim_matches('"').replace('-', "_"));
            match section {
                "[lib]" => lib = v,
                "[package]" => package = v,
                _ => {}
            }
        }
    }
    let has_lib = root.join("src/lib.rs").is_file() || toml.lines().any(|l| l.trim() == "[lib]");
    if has_lib { lib.or(package) } else { None }
}

/// Warns about impl references whose file or function can't be found.
fn check_impls(out: &Path, output: &Output, keyword: &str) {
    for i in &output.impls {
        let path = out.join(&i.file);
        let problem = match std::fs::read_to_string(&path) {
            Err(_) => format!("{} does not exist", path.display()),
            // `fn f(`, or a generic `fn f<T>(` / `func F[T any](`.
            Ok(src) if !["(", "<", "["].iter().any(|open| src.contains(&format!("{keyword} {}{open}", i.function))) => {
                format!("{} does not define {keyword} {}", path.display(), i.function)
            }
            Ok(_) => continue,
        };
        eprintln!("warning: {} is implemented by {}:{}, but {problem}", i.node, i.file, i.function);
        eprintln!("         expected: {}", i.signature);
    }
}
