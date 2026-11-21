mod serve;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use vandorf::codegen::go::GoOptions;
use vandorf::codegen::rust::RustOptions;
use vandorf::codegen::{self, Output};

const USAGE: &str = "\
usage:
  vandorf check <file.sys>
  vandorf doc   <file.sys> [-o out.json]
  vandorf go    <file.sys> -o <dir> [--package name] [--module import/path/of/dir] [--types-dir types]
  vandorf rust  <file.sys> -o <dir> [--module-dir path/under/src]
  vandorf serve [--addr 127.0.0.1:8080] [--dir .]   # editor + visualizer workbench

Each backend writes two artifacts: the types (typegen) and the procs (codegen).
Impl paths in `= \"file:function\"` are relative to <dir>; each backend uses the
reference with its extension (.go / .rs).

go:   <dir> is the code package; types go to <dir>/<types-dir> as their own
      package (`.` keeps them in the code package). Import paths come from the
      enclosing go.mod, or --module (the import path of <dir>).
rust: <dir> is a module (<dir>/mod.rs, types.rs, procs.rs). Its place in the
      crate comes from the enclosing Cargo.toml, or --module-dir (relative to
      src/).";

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
    input: PathBuf,
    out: Option<PathBuf>,
    package: Option<String>,
    module: Option<String>,
    types_dir: Option<String>,
    module_dir: Option<String>,
}

fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut it = argv.into_iter();
    let mut args = Args { cmd: it.next().ok_or(USAGE)?, ..Default::default() };
    let mut input = None;
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{a} needs a value\n\n{USAGE}"));
        match a.as_str() {
            "-o" | "--out" => args.out = Some(PathBuf::from(val()?)),
            "--package" => args.package = Some(val()?),
            "--module" => args.module = Some(val()?),
            "--types-dir" => args.types_dir = Some(val()?),
            "--module-dir" => args.module_dir = Some(val()?),
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with('-') => return Err(format!("unknown flag {s}\n\n{USAGE}")),
            _ if input.is_none() => input = Some(PathBuf::from(&a)),
            _ => return Err(format!("unexpected argument {a}\n\n{USAGE}")),
        }
    }
    args.input = input.ok_or(USAGE)?;
    Ok(args)
}

fn run(argv: Vec<String>) -> Result<(), String> {
    if argv.first().map(String::as_str) == Some("serve") {
        return serve_cmd(&argv[1..]);
    }
    let args = parse_args(argv)?;
    let src = std::fs::read_to_string(&args.input)
        .map_err(|e| format!("cannot read {}: {e}", args.input.display()))?;
    let prog = vandorf::compile(&args.input.display().to_string(), &src)?;

    match args.cmd.as_str() {
        "check" => {
            eprintln!("ok: {} procs", prog.all_procs().len());
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
        "go" => {
            let out = out_dir(&args)?;
            let mut opts = GoOptions::new(
                args.package.clone().unwrap_or_else(|| default_package(&out)),
                args.module.clone().or_else(|| go_import_path(&out)),
            );
            if let Some(d) = &args.types_dir {
                opts.types_dir = d.clone();
            }
            let output = codegen::go::generate(&prog, &opts)?;
            let written = write_output(&out, &output)?;
            // Generated code is already indented; gofmt only aligns it.
            format_with("gofmt", &["-w"], &written);
            check_impls(&out, &output, "func");
            Ok(())
        }
        "rust" => {
            let out = out_dir(&args)?;
            let opts = RustOptions { module_dir: args.module_dir.clone().or_else(|| rust_module_dir(&out)) };
            let output = codegen::rust::generate(&prog, &opts)?;
            let written = write_output(&out, &output)?;
            format_with("rustfmt", &["--edition", "2024"], &written);
            check_impls(&out, &output, "fn");
            Ok(())
        }
        other => Err(format!("unknown command {other}\n\n{USAGE}")),
    }
}

fn out_dir(args: &Args) -> Result<PathBuf, String> {
    let out = args.out.clone().ok_or_else(|| format!("{} needs -o <dir>\n\n{USAGE}", args.cmd))?;
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(out)
}

/// Writes both artifacts, returning the paths written.
fn write_output(out: &Path, output: &Output) -> Result<Vec<PathBuf>, String> {
    let mut written = vec![];
    for (kind, art) in [("types", &output.types), ("code", &output.code)] {
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
    let (mut addr, mut dir) = ("127.0.0.1:8080".to_string(), PathBuf::from("."));
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        let val = it.next().ok_or_else(|| format!("{a} needs a value\n\n{USAGE}"));
        match a.as_str() {
            "--addr" => addr = val?.clone(),
            "--dir" => dir = PathBuf::from(val?),
            _ => return Err(format!("unknown argument {a}\n\n{USAGE}")),
        }
    }
    serve::serve(&addr, &dir)
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

/// Warns about impl references whose file or function can't be found.
fn check_impls(out: &Path, output: &Output, keyword: &str) {
    for i in &output.impls {
        let path = out.join(&i.file);
        let problem = match std::fs::read_to_string(&path) {
            Err(_) => format!("{} does not exist", path.display()),
            Ok(src) if !src.contains(&format!("{keyword} {}(", i.function)) => {
                format!("{} does not define {keyword} {}", path.display(), i.function)
            }
            Ok(_) => continue,
        };
        eprintln!("warning: {} is implemented by {}:{}, but {problem}", i.node, i.file, i.function);
        eprintln!("         expected: {}", i.signature);
    }
}
