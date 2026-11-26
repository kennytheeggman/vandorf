//! Implementations of vcc's I/O interfaces (io.sys): the only
//! impure part of vcc.

use crate::vcc::types::{List, Str};
use std::path::{Path, PathBuf};

/// Reads the file at path as text; fails, naming the path, if it can't.
pub fn read(path: Str) -> Result<Str, String> {
    std::fs::read_to_string(&*path)
        .map(Str::from)
        .map_err(|e| format!("cannot read {path}: {e}"))
}

/// Writes text to path, creating its directory first; fails, naming the path,
/// if it can't.
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

/// Reads the working directory, with / separators like the paths JoinP
/// builds.
pub fn cwd(_n: i64) -> Result<Str, String> {
    std::env::current_dir()
        .map(|d| Str::from(d.to_string_lossy().replace('\\', "/").as_str()))
        .map_err(|e| e.to_string())
}

/// Lists the .sys files under dir, recursively, skipping hidden directories
/// and build output (target, node_modules).
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

/// Formats the generated Rust files in place, if rustfmt is installed; does
/// nothing otherwise.
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

/// Formats the generated Go files in place, if gofmt is installed; does
/// nothing otherwise.
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

/// Runs cmd with the platform's shell (sh -c, or cmd /C on Windows) in dir,
/// on this process's terminal; Err with why if it can't start or exits
/// unsuccessfully.
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

/// Prints a line on stderr.
pub fn eprint(line: Str) -> Result<bool, String> {
    eprintln!("{line}");
    Ok(true)
}

/// Prints a command's output on stdout, ending the proc that called it.
pub fn print_out(text: Str) -> Result<(), String> {
    println!("{text}");
    Ok(())
}

/// Prints a command's closing note on stderr, ending the proc that called it.
pub fn eprint_note(line: Str) -> Result<(), String> {
    eprintln!("{line}");
    Ok(())
}

/// Changes the working directory to path (for -C), returning the error as a
/// value if it can't.
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

/// Finds the directory for temporary files ($TMPDIR, or the system's), with
/// / separators and no trailing one.
pub fn temp_dir(_n: i64) -> Result<Str, String> {
    let d = std::env::temp_dir().to_string_lossy().replace('\\', "/");
    let d = d.trim_end_matches('/');
    Ok(if d.is_empty() { "/" } else { d }.into())
}

/// Makes every file under dir read-only, leaving the directories writable so
/// the tree can still be removed; dir's .git is left alone.
pub fn read_only(dir: Str) -> Result<bool, String> {
    fn go(dir: &Path, top: bool) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for e in entries.flatten() {
            let (p, Ok(t)) = (e.path(), e.file_type()) else {
                continue;
            };
            if top && e.file_name() == ".git" {
                continue;
            }
            if t.is_dir() {
                go(&p, false)?;
            } else if t.is_file() {
                let fail = |e: std::io::Error| format!("{}: {e}", p.display());
                let mut perm = std::fs::metadata(&p).map_err(fail)?.permissions();
                perm.set_readonly(true);
                std::fs::set_permissions(&p, perm).map_err(fail)?;
            }
        }
        Ok(())
    }
    go(Path::new(&*dir), true)?;
    Ok(true)
}

/// Prints a line on stdout, for the command's output, and goes on.
pub fn print_line(line: Str) -> Result<bool, String> {
    println!("{line}");
    Ok(true)
}

/// Prints prompt on stderr, then waits for a line on stdin, or its end;
/// gives the line, without its line break.
pub fn wait_line(prompt: Str) -> Result<Str, String> {
    eprintln!("{prompt}");
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("cannot read stdin: {e}"))?;
    Ok(line.trim_end_matches(['\r', '\n']).into())
}
