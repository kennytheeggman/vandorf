//! Implementations of the IDE's interfaces (../sys/ide): the session, its
//! pages, files under the source directory, and terminals.

use crate::vcc::types::*;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

fn s(x: &str) -> Str {
    x.into()
}

fn err_str(msg: String) -> ResString {
    ResString::Err(Diag { msg: msg.into(), span: Span { at: 0, end: 0, file: 0 } })
}

// ---- Session ----

/// A random token for this run, which writes and terminals must present.
pub fn token(_n: i64) -> Result<Str, String> {
    static TOKEN: OnceLock<String> = OnceLock::new();
    Ok(s(TOKEN.get_or_init(|| {
        let mut b = [0u8; 16];
        let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
        b.iter().map(|x| format!("{x:02x}")).collect()
    })))
}

/// The source directory: the working directory, canonical.
pub fn base(_n: i64) -> Result<Str, String> {
    static BASE: OnceLock<String> = OnceLock::new();
    Ok(s(BASE.get_or_init(|| {
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        std::fs::canonicalize(&cwd).unwrap_or(cwd).to_string_lossy().into_owned()
    })))
}

/// An IDE page by name ("" if there's none).
pub fn page(name: Str) -> Result<Str, String> {
    Ok(s(match &*name {
        "ide" => include_str!("../../../web/ide.html"),
        "explorer" => include_str!("../../../web/explorer.html"),
        "viewer" => include_str!("../../../web/viewer.html"),
        "terminal" => include_str!("../../../web/terminal.html"),
        "workbench" => include_str!("../../../web/workbench.html"),
        "viz" => include_str!("../../../web/visualizer.html"),
        "project" => include_str!("../../../web/project.html"),
        "ui.css" => include_str!("../../../web/ui.css"),
        _ => "",
    }))
}

// ---- Files ----

pub fn canon(path: Str) -> Result<ResString, String> {
    Ok(match std::fs::canonicalize(&*path) {
        Ok(p) => ResString::Ok(s(&p.to_string_lossy())),
        Err(e) => err_str(e.to_string()),
    })
}

pub fn try_read(path: Str) -> Result<ResString, String> {
    Ok(match std::fs::read_to_string(&*path) {
        Ok(t) => ResString::Ok(t.into()),
        Err(e) => err_str(e.to_string()),
    })
}

pub fn try_write(path: Str, text: Str) -> Result<ResString, String> {
    Ok(match std::fs::write(&*path, &*text) {
        Ok(()) => ResString::Ok(s("")),
        Err(e) => err_str(e.to_string()),
    })
}

/// The .sys files under base, relative to it, sorted.
pub fn walk_rel(base: Str) -> Result<List<Str>, String> {
    fn go(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !name.starts_with('.') && name != "target" && name != "node_modules" {
                    go(&p, base, out);
                }
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sys"))
                && let Ok(rel) = p.strip_prefix(base)
            {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = vec![];
    go(Path::new(&*base), Path::new(&*base), &mut out);
    out.sort();
    Ok(out.iter().map(|p| s(p)).collect())
}

/// Parses a file, its failure a value: the message and where it is.
pub fn try_parse(src: Str, file: i64) -> Result<ResFile, String> {
    Ok(match crate::vcc::run_parse_file(src, file) {
        Ok(f) => ResFile::Ok(f),
        Err(e) => {
            // The innermost message is "at:end: what".
            let text = e.to_string();
            let (mut at, mut end, mut msg) = (0, 0, text.clone());
            for (i, _) in text.match_indices(": ") {
                let head = &text[..i];
                let start = head.rfind(' ').map_or(0, |j| j + 1);
                if let Some((a, b)) = head[start..].split_once(':')
                    && let (Ok(a), Ok(b)) = (a.parse(), b.parse())
                {
                    (at, end, msg) = (a, b, text[i + 2..].to_string());
                }
            }
            ResFile::Err(Diag { msg: msg.into(), span: Span { at, end, file } })
        }
    })
}

// ---- Terminals ----

/// Output kept per terminal, for reconnecting panels.
const SCROLLBACK: usize = 1 << 20;

struct Term {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    out: Mutex<TermOut>,
    ready: Condvar,
    pid: u32,
}

#[derive(Default)]
struct TermOut {
    data: Vec<u8>,
    /// Stream offset of data[0].
    start: u64,
    closed: bool,
}

fn terms() -> &'static Mutex<(HashMap<i64, Arc<Term>>, i64)> {
    static TERMS: OnceLock<Mutex<(HashMap<i64, Arc<Term>>, i64)>> = OnceLock::new();
    TERMS.get_or_init(|| Mutex::new((HashMap::new(), 1)))
}

fn get(id: i64) -> Option<Arc<Term>> {
    terms().lock().unwrap().0.get(&id).cloned()
}

/// `PATH` with this executable's directory first.
fn path_with_self() -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).into_iter().collect();
    dirs.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
    std::env::join_paths(dirs).unwrap_or_default()
}

/// A shell on a pty (via script(1)) in cwd; its id.
pub fn term_spawn(cwd: Str) -> Result<ResInt, String> {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/bash".into());
    let child = Command::new("script")
        .args(["-qfec", &format!("{shell} -i"), "/dev/null"])
        .current_dir(&*cwd)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env("PATH", path_with_self())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return Ok(ResInt::Err(Diag { msg: format!("cannot start a shell with script(1): {e}").into(), span: Span { at: 0, end: 0, file: 0 } })),
    };
    let (stdin, stdout, stderr) = (child.stdin.take().unwrap(), child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let t = Arc::new(Term { pid: child.id(), child: Mutex::new(child), stdin: Mutex::new(stdin), out: Mutex::new(TermOut::default()), ready: Condvar::new() });
    let pump = |t: Arc<Term>, mut r: Box<dyn Read + Send>, closes: bool| {
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                let n = r.read(&mut buf).unwrap_or(0);
                let mut o = t.out.lock().unwrap();
                if n == 0 {
                    if closes {
                        o.closed = true;
                        t.ready.notify_all();
                    }
                    return;
                }
                o.data.extend_from_slice(&buf[..n]);
                if o.data.len() > SCROLLBACK {
                    let cut = o.data.len() - SCROLLBACK;
                    o.data.drain(..cut);
                    o.start += cut as u64;
                }
                t.ready.notify_all();
            }
        });
    };
    pump(t.clone(), Box::new(stdout), true);
    pump(t.clone(), Box::new(stderr), false);
    let mut reg = terms().lock().unwrap();
    let id = reg.1;
    reg.1 += 1;
    reg.0.insert(id, t);
    Ok(ResInt::Ok(id))
}

pub fn term_list(_n: i64) -> Result<List<i64>, String> {
    let mut ids: Vec<i64> = terms().lock().unwrap().0.keys().copied().collect();
    ids.sort();
    Ok(ids.into())
}

/// Output from stream offset `from`, waiting up to 20 s for some. Only whole
/// UTF-8 characters: `next` is where the next read starts.
pub fn term_read(id: i64, from: i64) -> Result<TermChunk, String> {
    let Some(t) = get(id) else { return Ok(TermChunk { ok: false, data: s(""), next: 0, closed: false }) };
    let mut o = t.out.lock().unwrap();
    let from = from.max(0) as u64;
    let end = |o: &TermOut| o.start + o.data.len() as u64;
    if end(&o) <= from && !o.closed {
        o = t.ready.wait_timeout_while(o, Duration::from_secs(20), |o| end(o) <= from && !o.closed).unwrap().0;
    }
    let from = from.clamp(o.start, end(&o));
    let bytes = &o.data[(from - o.start) as usize..];
    let (text, used) = match std::str::from_utf8(bytes) {
        Ok(t) => (t.to_string(), bytes.len()),
        Err(e) if e.valid_up_to() > 0 => (String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned(), e.valid_up_to()),
        // An invalid byte: pass it on as a replacement character.
        Err(e) if e.error_len().is_some() => ("\u{fffd}".to_string(), e.error_len().unwrap()),
        // Only part of a character so far.
        Err(_) => (String::new(), 0),
    };
    Ok(TermChunk { ok: true, data: text.into(), next: (from + used as u64) as i64, closed: o.closed })
}

pub fn term_input(id: i64, data: Str) -> Result<ResString, String> {
    let Some(t) = get(id) else { return Ok(err_str("no such terminal".into())) };
    let mut w = t.stdin.lock().unwrap();
    Ok(match w.write_all(data.as_bytes()).and_then(|_| w.flush()) {
        Ok(()) => ResString::Ok(s("")),
        Err(e) => err_str(format!("terminal closed: {e}")),
    })
}

pub fn term_resize(id: i64, cols: i64, rows: i64) -> Result<ResString, String> {
    let Some(t) = get(id) else { return Ok(err_str("no such terminal".into())) };
    // The pty's slave: the shell's stdin, a child of script(1).
    let pty = std::fs::read_to_string(format!("/proc/{0}/task/{0}/children", t.pid))
        .ok()
        .and_then(|kids| kids.split_whitespace().next().map(str::to_string))
        .and_then(|kid| std::fs::read_link(format!("/proc/{kid}/fd/0")).ok())
        .filter(|p| p.starts_with("/dev/pts"));
    let Some(pty) = pty else { return Ok(err_str("terminal has no pty yet".into())) };
    Ok(match Command::new("stty").arg("-F").arg(&pty).args(["rows", &rows.to_string(), "cols", &cols.to_string()]).status() {
        Ok(st) if st.success() => ResString::Ok(s("")),
        Ok(_) => err_str("stty failed".into()),
        Err(e) => err_str(format!("stty: {e}")),
    })
}

pub fn term_kill(id: i64) -> Result<bool, String> {
    let Some(t) = terms().lock().unwrap().0.remove(&id) else { return Ok(false) };
    let _ = t.child.lock().unwrap().kill();
    Ok(true)
}
