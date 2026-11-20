use std::path::{Path, PathBuf};
use std::process::ExitCode;
use vandorf::codegen::go::{self, GoOptions};

const USAGE: &str = "\
usage:
  vandorf check <file.sys>
  vandorf doc   <file.sys> [-o out.json]
  vandorf go    <file.sys> -o <dir> [--package name] [--module import/path/of/dir]

go: impl paths in `= \"file:function\"` are relative to <dir>. Impls outside
<dir> are imported; their import path is derived from the enclosing go.mod,
or from --module (the import path of <dir> itself).";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    cmd: String,
    input: PathBuf,
    out: Option<PathBuf>,
    package: Option<String>,
    module: Option<String>,
}

fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut it = argv.into_iter();
    let cmd = it.next().ok_or(USAGE)?;
    let mut input = None;
    let (mut out, mut package, mut module) = (None, None, None);
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{a} needs a value\n\n{USAGE}"));
        match a.as_str() {
            "-o" | "--out" => out = Some(PathBuf::from(val()?)),
            "--package" => package = Some(val()?),
            "--module" => module = Some(val()?),
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with('-') => return Err(format!("unknown flag {s}\n\n{USAGE}")),
            _ if input.is_none() => input = Some(PathBuf::from(&a)),
            _ => return Err(format!("unexpected argument {a}\n\n{USAGE}")),
        }
    }
    let input = input.ok_or(USAGE)?;
    Ok(Args { cmd, input, out, package, module })
}

fn run(argv: Vec<String>) -> Result<(), String> {
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
            let out = args.out.ok_or_else(|| format!("go needs -o <dir>\n\n{USAGE}"))?;
            std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
            let package = args.package.unwrap_or_else(|| default_package(&out));
            let import_base = args.module.or_else(|| module_import_path(&out));
            let output = go::generate(&prog, &GoOptions { package, import_base })?;
            for (name, src) in &output.files {
                let p = out.join(name);
                std::fs::write(&p, src).map_err(|e| format!("{}: {e}", p.display()))?;
                eprintln!("wrote {}", p.display());
            }
            // Generated code is already indented; gofmt only aligns it.
            let paths: Vec<PathBuf> = output.files.keys().map(|n| out.join(n)).collect();
            if let Err(e) = std::process::Command::new("gofmt").arg("-w").args(&paths).status() {
                eprintln!("note: gofmt not run ({e})");
            }
            check_impls(&out, &output.impls);
            Ok(())
        }
        other => Err(format!("unknown command {other}\n\n{USAGE}")),
    }
}

fn default_package(dir: &Path) -> String {
    let base = std::fs::canonicalize(dir).ok()
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "generated".into());
    let s: String = base.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if s.starts_with(|c: char| c.is_ascii_digit()) { format!("p{s}") } else { s }
}

/// Import path of `dir`, from the nearest enclosing go.mod.
fn module_import_path(dir: &Path) -> Option<String> {
    let dir = std::fs::canonicalize(dir).ok()?;
    let mut root = dir.as_path();
    loop {
        if let Ok(gomod) = std::fs::read_to_string(root.join("go.mod")) {
            let module = gomod.lines().find_map(|l| l.trim().strip_prefix("module "))?.trim().trim_matches('"');
            let rel = dir.strip_prefix(root).ok()?;
            let mut path = module.to_string();
            for c in rel.components() {
                path.push('/');
                path.push_str(&c.as_os_str().to_string_lossy());
            }
            return Some(path);
        }
        root = root.parent()?;
    }
}

/// Warns about impl references whose file or function can't be found.
fn check_impls(out: &Path, impls: &[go::ImplUse]) {
    for i in impls {
        let path = out.join(&i.file);
        let problem = match std::fs::read_to_string(&path) {
            Err(_) => format!("{} does not exist", path.display()),
            Ok(src) if !src.contains(&format!("func {}(", i.function)) => {
                format!("{} does not define func {}", path.display(), i.function)
            }
            Ok(_) => continue,
        };
        eprintln!("warning: {} is implemented by {}:{}, but {problem}", i.node, i.file, i.function);
        eprintln!("         expected: {}", i.signature);
    }
}
