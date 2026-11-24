//! Implementations of vcc's I/O interfaces (io.sys): the only
//! impure part of vcc.

use crate::vcc::types::{List, Str};
use std::path::{Path, PathBuf};

pub fn read(path: Str) -> Result<Str, String> {
    std::fs::read_to_string(&*path)
        .map(Str::from)
        .map_err(|e| format!("cannot read {path}: {e}"))
}

/// Writes text to path, creating its directory.
pub fn write(path: Str, text: Str) -> Result<bool, String> {
    if let Some(dir) = Path::new(&*path).parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&*path, &*text).map_err(|e| format!("{path}: {e}"))?;
    Ok(true)
}

pub fn is_file(path: Str) -> Result<bool, String> {
    Ok(Path::new(&*path).is_file())
}

pub fn is_dir(path: Str) -> Result<bool, String> {
    Ok(Path::new(&*path).is_dir())
}

/// The working directory, with / separators like the paths JoinP builds.
pub fn cwd(_n: i64) -> Result<Str, String> {
    std::env::current_dir()
        .map(|d| Str::from(d.to_string_lossy().replace('\\', "/").as_str()))
        .map_err(|e| e.to_string())
}

/// The .sys files under dir, skipping hidden directories and build output.
pub fn walk(dir: Str) -> Result<List<Str>, String> {
    fn go(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !name.starts_with('.') && name != "target" && name != "node_modules" {
                    go(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "sys") {
                out.push(p);
            }
        }
    }
    let mut out = vec![];
    go(Path::new(&*dir), &mut out);
    Ok(out
        .iter()
        .map(|p| Str::from(p.to_string_lossy().as_ref()))
        .collect())
}

/// Formats Rust files, if rustfmt is installed.
pub fn rustfmt(files: List<Str>) -> Result<bool, String> {
    let paths: Vec<&str> = files.iter().map(|f| &**f).collect();
    match std::process::Command::new("rustfmt")
        .args(["--edition", "2024"])
        .args(&paths)
        .status()
    {
        Ok(s) if !s.success() => eprintln!("note: rustfmt failed ({s})"),
        Ok(_) => {}
        Err(e) => eprintln!("note: rustfmt not run ({e})"),
    }
    Ok(true)
}

/// Formats Go files, if gofmt is installed.
pub fn gofmt(files: List<Str>) -> Result<bool, String> {
    let paths: Vec<&str> = files.iter().map(|f| &**f).collect();
    match std::process::Command::new("gofmt")
        .arg("-w")
        .args(&paths)
        .status()
    {
        Ok(s) if !s.success() => eprintln!("note: gofmt failed ({s})"),
        Ok(_) => {}
        Err(e) => eprintln!("note: gofmt not run ({e})"),
    }
    Ok(true)
}

/// Runs cmd with the platform's shell (sh -c, or cmd /C on Windows) in
/// dir, on this process's terminal; Err with why if it fails.
pub fn shell(cmd: Str, dir: Str) -> Result<super::types::ResString, String> {
    use super::types::{Diag, ResString, Span};
    let mut c = std::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" });
    c.args(if cfg!(windows) {
        ["/C", &*cmd]
    } else {
        ["-c", &*cmd]
    });
    let fail = |msg: String| {
        ResString::Err(Diag {
            msg: msg.into(),
            span: Span {
                at: 0,
                end: 0,
                file: 0,
            },
        })
    };
    Ok(match c.current_dir(&*dir).status() {
        Ok(st) if st.success() => ResString::Ok(cmd),
        Ok(st) => fail(st.to_string()),
        Err(e) => fail(e.to_string()),
    })
}

/// A line on stderr.
pub fn eprint(line: Str) -> Result<bool, String> {
    eprintln!("{line}");
    Ok(true)
}

/// A command's output, on stdout, and a closing note on stderr; each ends
/// a proc that returns nothing.
pub fn print_out(text: Str) -> Result<(), String> {
    println!("{text}");
    Ok(())
}

pub fn eprint_note(line: Str) -> Result<(), String> {
    eprintln!("{line}");
    Ok(())
}

/// Changes the working directory (for -C).
pub fn chdir(path: Str) -> Result<super::types::ResString, String> {
    use super::types::{Diag, ResString, Span};
    Ok(match std::env::set_current_dir(&*path) {
        Ok(()) => ResString::Ok(path),
        Err(e) => ResString::Err(Diag {
            msg: format!("cannot use -C {path}: {e}").into(),
            span: Span {
                at: 0,
                end: 0,
                file: 0,
            },
        }),
    })
}
