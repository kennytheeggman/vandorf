//! Implementations of the IDE's interfaces (ide.sys): the session, its
//! pages, files under the source directory, and terminals.

use crate::vcc::types::*;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

fn s(x: &str) -> Str {
    x.into()
}

/// p with / separators, like the paths JoinP and Parent build.
fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn err_str(msg: String) -> ResString {
    ResString::Err(Diag {
        msg: msg.into(),
        span: Span {
            at: 0,
            end: 0,
            file: 0,
        },
    })
}

// ---- Session ----

/// A random token for this run, which writes and terminals must present.
pub fn token(_n: i64) -> Result<Str, String> {
    static TOKEN: OnceLock<String> = OnceLock::new();
    Ok(s(TOKEN.get_or_init(|| {
        let mut b = [0u8; 16];
        let _ = getrandom::fill(&mut b);
        b.iter().map(|x| format!("{x:02x}")).collect()
    })))
}

/// The source directory: the working directory, canonical.
pub fn base(_n: i64) -> Result<Str, String> {
    static BASE: OnceLock<String> = OnceLock::new();
    Ok(s(BASE.get_or_init(|| {
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        slash(&dunce::canonicalize(&cwd).unwrap_or(cwd))
    })))
}

/// An IDE page by name ("" if there's none).
pub fn page(name: Str) -> Result<Str, String> {
    Ok(s(match &*name {
        "ide" => include_str!("../../web/ide.html"),
        "explorer" => include_str!("../../web/explorer.html"),
        "viewer" => include_str!("../../web/viewer.html"),
        "terminal" => include_str!("../../web/terminal.html"),
        "workbench" => include_str!("../../web/workbench.html"),
        "viz" => include_str!("../../web/visualizer.html"),
        "project" => include_str!("../../web/project.html"),
        "ui.css" => include_str!("../../web/ui.css"),
        _ => "",
    }))
}

// ---- Files ----

pub fn canon(path: Str) -> Result<ResString, String> {
    Ok(match dunce::canonicalize(&*path) {
        Ok(p) => ResString::Ok(s(&slash(&p))),
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
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
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
            ResFile::Err(Diag {
                msg: msg.into(),
                span: Span { at, end, file },
            })
        }
    })
}

// ---- Checks ----

/// Checks kept, most recently used first.
const CHECKS_KEPT: usize = 8;

/// How long a request waits for another's check before making its own (if
/// that one never puts it, say because its proc failed).
const CLAIM_WAIT: Duration = Duration::from_secs(60);

#[derive(Default)]
struct CheckStore {
    done: Vec<(Str, UnitCheck)>,
    /// Keys a request is making a check for.
    making: Vec<Str>,
}

fn checks() -> &'static (Mutex<CheckStore>, Condvar) {
    static CHECKS: OnceLock<(Mutex<CheckStore>, Condvar)> = OnceLock::new();
    CHECKS.get_or_init(|| (Mutex::new(CheckStore::default()), Condvar::new()))
}

/// The check of key, kept or once made by another request; None when this
/// request is to make it (or has waited too long for another's).
pub fn check_claim(key: Str) -> Result<MaybeCheck, String> {
    let (store, ready) = checks();
    let mut cs = store.lock().unwrap();
    let deadline = std::time::Instant::now() + CLAIM_WAIT;
    loop {
        if let Some(i) = cs.done.iter().position(|(k, _)| *k == key) {
            let hit = cs.done.remove(i);
            cs.done.insert(0, hit.clone());
            return Ok(MaybeCheck::Some(hit.1));
        }
        if !cs.making.contains(&key) {
            cs.making.push(key);
            return Ok(MaybeCheck::None);
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            return Ok(MaybeCheck::None);
        }
        cs = ready.wait_timeout(cs, deadline - now).unwrap().0;
    }
}

pub fn check_put(key: Str, c: UnitCheck) -> Result<bool, String> {
    let (store, ready) = checks();
    let mut cs = store.lock().unwrap();
    cs.making.retain(|k| *k != key);
    cs.done.retain(|(k, _)| *k != key);
    cs.done.insert(0, (key, c));
    cs.done.truncate(CHECKS_KEPT);
    ready.notify_all();
    Ok(true)
}

// ---- Terminals ----

/// Output kept per terminal, for reconnecting panels.
const SCROLLBACK: usize = 1 << 20;

struct Term {
    cwd: Mutex<String>,
    #[cfg(target_os = "linux")]
    pid: Option<u32>,
    /// The pty's master, for resizing; None once the shell has exited.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    out: Mutex<TermOut>,
    ready: Condvar,
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
    let mut dirs: Vec<PathBuf> = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .into_iter()
        .collect();
    dirs.extend(
        std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default(),
    );
    std::env::join_paths(dirs).unwrap_or_default()
}

/// `$SHELL`, or `%COMSPEC%` on Windows. On a pty it's interactive.
fn shell() -> CommandBuilder {
    let (var, fallback) = if cfg!(windows) {
        ("COMSPEC", "cmd.exe")
    } else {
        ("SHELL", "/bin/bash")
    };
    CommandBuilder::new(
        std::env::var(var)
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback.into()),
    )
}

fn spawn(cwd: &str) -> Result<Arc<Term>, String> {
    let pty = native_pty_system()
        .openpty(PtySize::default())
        .map_err(|e| format!("cannot open a pty: {e}"))?;
    let mut cmd = shell();
    cmd.cwd(cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("PATH", path_with_self());
    let mut child = pty
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("cannot start a shell: {e}"))?;
    // Only the shell holds the slave now, so output ends when it exits.
    drop(pty.slave);
    let mut reader = pty
        .master
        .try_clone_reader()
        .map_err(|e| format!("cannot read the pty: {e}"))?;
    let writer = pty
        .master
        .take_writer()
        .map_err(|e| format!("cannot write the pty: {e}"))?;
    let t = Arc::new(Term {
        cwd: Mutex::new(cwd.into()),
        #[cfg(target_os = "linux")]
        pid: child.process_id(),
        master: Mutex::new(Some(pty.master)),
        writer: Mutex::new(writer),
        killer: Mutex::new(child.clone_killer()),
        out: Mutex::new(TermOut::default()),
        ready: Condvar::new(),
    });
    let pump = t.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n = reader.read(&mut buf).unwrap_or(0);
            let mut o = pump.out.lock().unwrap();
            if n == 0 {
                o.closed = true;
                pump.ready.notify_all();
                return;
            }
            o.data.extend_from_slice(&buf[..n]);
            if o.data.len() > SCROLLBACK {
                let cut = o.data.len() - SCROLLBACK;
                o.data.drain(..cut);
                o.start += cut as u64;
            }
            pump.ready.notify_all();
        }
    });
    // ConPTY keeps the output open past the shell's exit until the master
    // closes.
    let waiter = t.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        waiter.master.lock().unwrap().take();
    });
    Ok(t)
}

/// The user's shell on a pty in cwd; its id.
pub fn term_spawn(cwd: Str) -> Result<ResInt, String> {
    let t = match spawn(&cwd) {
        Ok(t) => t,
        Err(msg) => {
            return Ok(ResInt::Err(Diag {
                msg: msg.into(),
                span: Span {
                    at: 0,
                    end: 0,
                    file: 0,
                },
            }));
        }
    };
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
    let Some(t) = get(id) else {
        return Ok(TermChunk {
            ok: false,
            data: s(""),
            next: 0,
            closed: false,
            cwd: s(""),
        });
    };
    let mut o = t.out.lock().unwrap();
    let from = from.max(0) as u64;
    let end = |o: &TermOut| o.start + o.data.len() as u64;
    if end(&o) <= from && !o.closed {
        o = t
            .ready
            .wait_timeout_while(o, Duration::from_secs(20), |o| end(o) <= from && !o.closed)
            .unwrap()
            .0;
    }
    let from = from.clamp(o.start, end(&o));
    let bytes = &o.data[(from - o.start) as usize..];
    let (text, used) = match std::str::from_utf8(bytes) {
        Ok(t) => (t.to_string(), bytes.len()),
        Err(e) if e.valid_up_to() > 0 => (
            String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned(),
            e.valid_up_to(),
        ),
        // An invalid byte: pass it on as a replacement character.
        Err(e) if e.error_len().is_some() => ("\u{fffd}".to_string(), e.error_len().unwrap()),
        // Only part of a character so far.
        Err(_) => (String::new(), 0),
    };
    Ok(TermChunk {
        ok: true,
        data: text.into(),
        next: (from + used as u64) as i64,
        closed: o.closed,
        cwd: t.cwd(o.closed).into(),
    })
}

impl Term {
    fn cwd(&self, closed: bool) -> String {
        let mut cwd = self.cwd.lock().unwrap();
        if !closed && let Some(path) = self.live_cwd() {
            *cwd = path;
        }
        cwd.clone()
    }

    fn live_cwd(&self) -> Option<String> {
        #[cfg(target_os = "linux")]
        {
            let pid = self.pid?;
            let path = std::fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
            Some(path.to_string_lossy().into_owned())
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }
}

pub fn term_input(id: i64, data: Str) -> Result<ResString, String> {
    let Some(t) = get(id) else {
        return Ok(err_str("no such terminal".into()));
    };
    let mut w = t.writer.lock().unwrap();
    Ok(match w.write_all(data.as_bytes()).and_then(|_| w.flush()) {
        Ok(()) => ResString::Ok(s("")),
        Err(e) => err_str(format!("terminal closed: {e}")),
    })
}

pub fn term_resize(id: i64, cols: i64, rows: i64) -> Result<ResString, String> {
    let Some(t) = get(id) else {
        return Ok(err_str("no such terminal".into()));
    };
    let master = t.master.lock().unwrap();
    let Some(master) = master.as_ref() else {
        return Ok(err_str("terminal closed".into()));
    };
    let size = PtySize {
        rows: rows.clamp(1, u16::MAX as i64) as u16,
        cols: cols.clamp(1, u16::MAX as i64) as u16,
        ..PtySize::default()
    };
    // The pty signals SIGWINCH to its foreground job.
    Ok(match master.resize(size) {
        Ok(()) => ResString::Ok(s("")),
        Err(e) => err_str(format!("cannot resize the terminal: {e}")),
    })
}

pub fn term_kill(id: i64, keep: bool) -> Result<bool, String> {
    let term = if keep {
        get(id)
    } else {
        terms().lock().unwrap().0.remove(&id)
    };
    let Some(t) = term else {
        return Ok(false);
    };
    if !t.out.lock().unwrap().closed {
        let _ = t.killer.lock().unwrap().kill();
    }
    Ok(true)
}

/// Whether the server listens on a loopback address.
pub fn loopback(_n: i64) -> Result<bool, String> {
    let addr = super::http::addr();
    let host = addr.rsplit_once(':').map_or(addr, |(h, _)| h);
    Ok(super::ide_leaf::is_loopback(host))
}
