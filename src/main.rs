mod serve;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use vandorf::codegen::go::GoOptions;
use vandorf::codegen::rust::RustOptions;
use vandorf::codegen::{self, MainSpec, Output};
use vandorf::project::{self, Project};
use vandorf::sema::Program;

const USAGE: &str = "\
usage:
  vandorf build [<bin>]        # libraries, and every bin or just <bin>
  vandorf test                 # build, then each language's test commands
  vandorf check
  vandorf doc                  # the visualizer document, on stdout
  vandorf serve [--addr 127.0.0.1:8080]   # the IDE, on the working directory

Each works on the project: the nearest vandorf.toml in the current directory
or its parents (see the README for its fields). A project's .sys files share
one namespace. -C <dir>, anywhere in a command, runs it as if started in <dir>.

build generates what the manifest configures: [go] and [rust] libraries (the
types and the procs), an executable per [bin.<name>] running its entry proc
with the command-line arguments, and per [http.<name>] a Rust server (which
takes --addr to listen elsewhere). Impl paths in `= \"file:function\"` are
relative to the .sys file naming them; each backend uses the reference with
its extension (.go / .rs). After generating a language, build runs that
language's `post` commands; test runs each language's `test` commands. They
run in the manifest's directory; the first that fails fails the command.

serve offers terminals only when listening on a loopback address.";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        // Rendered diagnostics start with their level; other failures get one.
        Err(e) if e.starts_with("error: ") => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Takes out `-C <dir>` (anywhere, any number of times; each relative to the
/// last) and changes into it.
fn change_dir(argv: Vec<String>) -> Result<Vec<String>, String> {
    let mut rest = vec![];
    let mut it = argv.into_iter();
    while let Some(a) = it.next() {
        if a != "-C" {
            rest.push(a);
            continue;
        }
        let dir = it.next().ok_or_else(|| format!("-C needs a value (see vandorf --help)"))?;
        std::env::set_current_dir(&dir).map_err(|e| format!("cannot use -C {dir}: {e}"))?;
    }
    Ok(rest)
}

/// The project around the working directory, checked.
fn load() -> Result<(Project, Program), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let manifest = project::find(&cwd).ok_or_else(|| format!("no {} in {} or its parents", project::MANIFEST, cwd.display()))?;
    let p = project::load(&manifest)?;
    let prog = vandorf::compile_files(&p.files)?;
    Ok((p, prog))
}

fn run(argv: Vec<String>) -> Result<(), String> {
    let argv = change_dir(argv)?;
    let (cmd, args) = argv.split_first().ok_or(USAGE)?;
    if let Some(flag) = args.iter().find(|a| a.starts_with('-')).filter(|_| cmd != "serve") {
        return Err(format!("unknown flag {flag} (see vandorf --help)"));
    }
    match (cmd.as_str(), args) {
        ("-h" | "--help" | "help", _) => {
            println!("{USAGE}");
            Ok(())
        }
        ("serve", args) => serve_cmd(args),
        ("check", []) => {
            let (p, prog) = load()?;
            eprintln!("ok: {} files, {} procs", p.files.len(), prog.all_procs().len());
            Ok(())
        }
        ("doc", []) => {
            let (_, prog) = load()?;
            println!("{}", serde_json::to_string_pretty(&vandorf::document::generate(&prog)).unwrap());
            Ok(())
        }
        ("build", [] | [_]) => {
            let (p, prog) = load()?;
            build(&p, prog, args.first().map(String::as_str))
        }
        ("test", []) => {
            let (p, prog) = load()?;
            build(&p, prog, None)?;
            test(&p)
        }
        ("check" | "doc" | "build" | "test", _) => Err(format!("too many arguments for {cmd} (see vandorf --help)")),
        (other, _) => Err(format!("unknown command {other} (see vandorf --help)")),
    }
}

/// Generates what the manifest configures: every bin, or just `only`.
fn build(p: &Project, mut prog: Program, only: Option<&str>) -> Result<(), String> {
    let m = &p.manifest;
    if m.go.is_none() && m.rust.is_none() {
        return Err(format!("{}: configure [go] and/or [rust] to build", project::MANIFEST));
    }
    let bins: Vec<(&String, &project::Bin)> = match only {
        None => m.bin.iter().collect(),
        Some(name) => match m.bin.get_key_value(name) {
            Some(b) => vec![b],
            None => {
                let known: Vec<&str> = m.bin.keys().map(String::as_str).collect();
                return Err(format!("{}: no [bin.{name}]; bins: {}", project::MANIFEST, if known.is_empty() { "none".into() } else { known.join(", ") }));
            }
        },
    };
    if !m.http.is_empty() && m.rust.is_none() {
        return Err(format!("{}: [http.*] servers are generated in Rust: configure [rust]", project::MANIFEST));
    }
    if !m.http.is_empty() && m.go.is_some() {
        eprintln!("note: [http.*] servers are generated in Rust only, for now");
    }
    let dirs = p.dirs();
    if let Some(g) = &m.go {
        let out = p.root.join(&g.out);
        let output = go(&mut prog, &dirs, &out, g.package.clone(), g.module.clone(), g.types_dir.clone(), go_mains(p, g, &bins))?;
        check_impls(p, &prog, &out, &output, "func");
        commands(p, "post-build", &g.post)?;
    }
    if let Some(r) = &m.rust {
        let servers: Vec<(&String, &project::Server)> = if only.is_none() { m.http.iter().collect() } else { vec![] };
        let out = p.root.join(&r.out);
        let output = rust(&mut prog, &dirs, &out, r.module_dir.clone(), rust_mains(&bins), &servers, &p.root)?;
        check_impls(p, &prog, &out, &output, "fn");
        commands(p, "post-build", &r.post)?;
    }
    Ok(())
}

/// Runs each language's `test` commands, Go's then Rust's.
fn test(p: &Project) -> Result<(), String> {
    let m = &p.manifest;
    let (go, rust) = (m.go.as_ref().map_or(&[][..], |g| &g.test), m.rust.as_ref().map_or(&[][..], |r| &r.test));
    if go.is_empty() && rust.is_empty() {
        return Err(format!("{}: no tests: add test = [..] to [go] or [rust]", project::MANIFEST));
    }
    commands(p, "test", go)?;
    commands(p, "test", rust)
}

/// Runs shell commands in order, in the manifest's directory; the first
/// that fails fails. `what` names them in messages.
fn commands(p: &Project, what: &str, cmds: &[String]) -> Result<(), String> {
    for cmd in cmds {
        eprintln!("{what}: {cmd}");
        let failed = match shell(cmd).current_dir(&p.root).status() {
            Ok(st) if st.success() => continue,
            Ok(st) => st.to_string(),
            Err(e) => e.to_string(),
        };
        return Err(format!("{what} `{cmd}` failed ({failed})"));
    }
    Ok(())
}

/// `cmd`, run by the platform's shell.
fn shell(cmd: &str) -> std::process::Command {
    let mut c = std::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" });
    c.args(if cfg!(windows) { ["/C", cmd] } else { ["-c", cmd] });
    c
}

fn go(
    prog: &mut Program,
    dirs: &[PathBuf],
    out: &Path,
    package: Option<String>,
    module: Option<String>,
    types_dir: Option<String>,
    mains: Vec<MainSpec>,
) -> Result<Output, String> {
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
    Ok(output)
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

fn rust(
    prog: &mut Program,
    dirs: &[PathBuf],
    out: &Path,
    module_dir: Option<String>,
    mains: Vec<MainSpec>,
    servers: &[(&String, &project::Server)],
    root: &Path,
) -> Result<Output, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    project::rebase_impls(prog, dirs, out);
    let module_dir = module_dir.or_else(|| rust_module_dir(out));
    let servers = servers.iter().map(|(name, s)| server_spec(name, s, root, out, module_dir.as_deref())).collect::<Result<_, _>>()?;
    let opts = RustOptions { module_dir, mains, crate_name: rust_lib_name(out), servers };
    let output = codegen::rust::generate(prog, &opts)?;
    let written = write_output(out, &output)?;
    format_with("rustfmt", &["--edition", "2024"], &written);
    Ok(output)
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
    Ok(codegen::rust::ServerSpec { name: name.into(), addr: s.addr.clone(), setup: s.setup.clone(), routes, raw, statics, path })
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
    let mut addr = "127.0.0.1:8080".to_string();
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--addr" => addr = it.next().ok_or_else(|| format!("--addr needs a value (see vandorf --help)"))?.clone(),
            _ => return Err(format!("unknown argument {a} for serve (see vandorf --help)")),
        }
    }
    serve::serve(serve::Options { addr, dir: PathBuf::from(".") })
}

fn default_package(dir: &Path) -> String {
    let base = dunce::canonicalize(dir).ok()
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "generated".into());
    let s: String = base.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if s.starts_with(|c: char| c.is_ascii_digit()) { format!("p{s}") } else { s }
}

/// Finds the nearest ancestor of `dir` containing `marker`, returning it and
/// `dir` relative to it as `/`-separated segments.
fn find_root(dir: &Path, marker: &str) -> Option<(PathBuf, Vec<String>)> {
    let dir = dunce::canonicalize(dir).ok()?;
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

/// Warns about impl references whose file or function can't be found, at
/// the func's declaration. Paths are relative to the project's root.
fn check_impls(p: &Project, prog: &Program, out: &Path, output: &Output, keyword: &str) {
    let out = project::normalize(&project::absolute(out));
    for i in &output.impls {
        let path = project::normalize(&out.join(&i.file));
        let shown = project::slash(&project::relative(&path, &p.root));
        let msg = match std::fs::read_to_string(&path) {
            Err(_) => format!("{} is implemented in {shown}, which does not exist", i.node),
            // `fn f(`, or a generic `fn f<T>(` / `func F[T any](`.
            Ok(src) if !["(", "<", "["].iter().any(|open| src.contains(&format!("{keyword} {}{open}", i.function))) => {
                format!("{} is implemented by {keyword} {} in {shown}, which does not define it", i.node, i.function)
            }
            Ok(_) => continue,
        };
        let n = &prog.nodes[i.id];
        let f = &p.files[n.span.context];
        let note = format!("expected: {}", i.signature);
        eprintln!("{}\n", vandorf::report::snippet("warning", &msg, &f.path, &f.text, n.span.into_range(), &[note]));
    }
}
