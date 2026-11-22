//! `vandorf serve`: a local IDE (explorer, tabbed diagram/text viewer,
//! terminal) over the source directory.
//!
//! Pages (each IDE panel is its own page, embedded as an iframe):
//! - `GET /`              the IDE shell
//! - `GET /explorer`      file explorer (files as directories of their procs)
//! - `GET /viewer`        one editor tab: diagram or text of a file, preview drawer
//! - `GET /viz`           the diagram renderer (also usable standalone)
//! - `GET /terminal`      the terminal panel
//! - `GET /workbench`     the old single-page workbench
//!
//! API:
//! - `GET /api/sources`   `.sys` files under the source dir
//! - `GET /api/tree`      every `.sys` file with its outline (procs, funcs, types)
//! - `GET /api/source?path=..`  one file's text (any file under the source dir)
//! - `POST /api/compile`  `{source}` or `{path}` -> document, symbols and
//!   generated code, or diagnostics. Same response shape as the Go
//!   visualizer server, plus `symbols`.
//! - `POST /api/save`     `{path, source}`: writes an existing `.sys` file, or
//!   an existing `vandorf.toml` if the new text is a valid manifest
//! - `POST /api/term`, `GET /api/term/read?id=&from=`, `POST /api/term/input?id=`,
//!   `POST /api/term/resize?id=&cols=&rows=`, `POST /api/term/kill?id=`:
//!   shells on a pty (via `script(1)`), read by long polling.
//!
//! Writes and terminals need the per-run token the pages are served with
//! (`X-Vandorf-Token`), so other sites can't drive them, and, on a loopback
//! address, a loopback `Host`, against DNS rebinding.

use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tiny_http::{Header, Method, Request, Response, Server};
use vandorf::codegen::go::GoOptions;
use vandorf::codegen::rust::RustOptions;
use vandorf::codegen::{self, MainSpec, Output};
use vandorf::project;
use vandorf::sema::{NodeKind, Program};

const IDE: &str = include_str!("../web/ide.html");
const EXPLORER: &str = include_str!("../web/explorer.html");
const VIEWER: &str = include_str!("../web/viewer.html");
const TERMINAL: &str = include_str!("../web/terminal.html");
const PROJECT: &str = include_str!("../web/project.html");
const WORKBENCH: &str = include_str!("../web/workbench.html");
const VISUALIZER: &str = include_str!("../web/visualizer.html");
const UI_CSS: &str = include_str!("../web/ui.css");

pub struct Options {
    pub addr: String,
    pub dir: PathBuf,
    /// Whether to offer terminals. Defaults to on for loopback addresses only.
    pub term: Option<bool>,
}

struct State {
    base: PathBuf,
    token: String,
    /// Bound to a loopback address: requests must name a loopback host.
    loopback: bool,
    term: bool,
    terms: Mutex<HashMap<u32, Arc<Term>>>,
    next_term: AtomicU32,
}

pub fn serve(opts: Options) -> Result<(), String> {
    let base = std::fs::canonicalize(&opts.dir).map_err(|e| format!("{}: {e}", opts.dir.display()))?;
    let server = Server::http(&opts.addr).map_err(|e| format!("cannot listen on {}: {e}", opts.addr))?;
    let host = opts.addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(&opts.addr);
    let loopback = is_loopback(host);
    let term = opts.term.unwrap_or(loopback);
    if term && !loopback {
        eprintln!("warning: terminals are enabled on a non-loopback address; anyone who can load the page gets a shell");
    }
    let state = Arc::new(State {
        base,
        token: token()?,
        loopback,
        term,
        terms: Mutex::new(HashMap::new()),
        next_term: AtomicU32::new(1),
    });
    eprintln!(
        "vandorf IDE on http://{}  (sources: {}{})",
        opts.addr,
        state.base.display(),
        if term { "" } else { ", terminal off" }
    );
    for req in server.incoming_requests() {
        let state = state.clone();
        // Terminal reads block, so every request gets its own thread.
        std::thread::spawn(move || handle(req, &state));
    }
    Ok(())
}

fn is_loopback(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h == "localhost" || h == "::1" || h.starts_with("127.")
}

fn token() -> Result<String, String> {
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| format!("cannot make a session token: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn header_value<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}

/// Whether a request may write files or drive a terminal.
fn authorized(req: &Request, st: &State) -> bool {
    let host_ok = !st.loopback
        || header_value(req, "Host").is_some_and(|h| {
            let name = match h.rsplit_once(':') {
                Some((n, p)) if p.chars().all(|c| c.is_ascii_digit()) => n,
                _ => h,
            };
            is_loopback(name)
        });
    host_ok && header_value(req, "X-Vandorf-Token") == Some(st.token.as_str())
}

fn handle(mut req: Request, st: &State) {
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let base = st.base.as_path();
    let page = |s: &str| html(s, st);
    let resp = match (req.method(), path) {
        (Method::Get, "/") => page(IDE),
        (Method::Get, "/explorer") => page(EXPLORER),
        (Method::Get, "/viewer") => page(VIEWER),
        (Method::Get, "/terminal") => page(TERMINAL),
        (Method::Get, "/project") => page(PROJECT),
        (Method::Get, "/workbench") => page(WORKBENCH),
        (Method::Get, "/viz" | "/viz/") => page(VISUALIZER),
        (Method::Get, "/ui.css") => Response::from_string(UI_CSS)
            .with_header(header("text/css; charset=utf-8"))
            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
            .boxed(),
        (Method::Get, "/api/sources") => api(list_sources(base).map(|files| {
            json!({ "ok": true, "root": base.display().to_string(), "files": files })
        })),
        (Method::Get, "/api/tree") => api(tree(base)),
        (Method::Get, "/api/project") => api(query_param(query, "dir")
            .ok_or_else(|| "missing ?dir=".to_string())
            .and_then(|d| project_info(base, &d))),
        (Method::Get, "/api/source") => api(query_param(query, "path")
            .ok_or_else(|| "missing ?path=".to_string())
            .and_then(|p| read_source(base, &p).map(|src| json!({ "ok": true, "path": p, "source": src })))),
        (Method::Post, "/api/compile") => api(body(&mut req).and_then(|b| compile(base, &b))),
        (Method::Post, "/api/save") if authorized(&req, st) => api(body(&mut req).and_then(|b| save(base, &b))),
        (Method::Post, "/api/save") => forbidden(),
        (_, p) if p.starts_with("/api/term") => {
            if !st.term {
                api(Err("terminals are off (start `vandorf serve --term` to enable them)".into()))
            } else if !authorized(&req, st) {
                forbidden()
            } else {
                term_api(&mut req, st, p, query)
            }
        }
        _ => Response::from_string("not found").with_status_code(404).boxed(),
    };
    let _ = req.respond(resp);
}

fn body(req: &mut Request) -> Result<String, String> {
    let mut body = String::new();
    req.as_reader().take(4 << 20).read_to_string(&mut body).map_err(|e| format!("bad request: {e}"))?;
    Ok(body)
}

fn forbidden() -> tiny_http::ResponseBox {
    Response::from_string(json!({ "ok": false, "error": "forbidden: missing or wrong session token" }).to_string())
        .with_status_code(403)
        .with_header(header("application/json"))
        .boxed()
}

/// Pages carry the session token, for the requests that need it.
fn html(s: &str, st: &State) -> tiny_http::ResponseBox {
    Response::from_string(s.replace("__VANDORF_TOKEN__", &st.token))
        .with_header(header("text/html; charset=utf-8"))
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
        .boxed()
}

fn api(r: Result<Value, String>) -> tiny_http::ResponseBox {
    let (code, v) = match r {
        Ok(v) => (200, v),
        Err(e) => (400, json!({ "ok": false, "error": e })),
    };
    Response::from_string(v.to_string())
        .with_status_code(code)
        .with_header(header("application/json"))
        .boxed()
}

fn header(ct: &str) -> Header {
    Header::from_bytes("Content-Type", ct).unwrap()
}

/* ---------- explorer ---------- */

/// Every `.sys` file with the project it belongs to and its outline.
fn tree(base: &Path) -> Result<Value, String> {
    let files = list_sources(base)?;
    let mut projects: HashMap<PathBuf, Value> = HashMap::new();
    let out: Vec<Value> = files
        .iter()
        .map(|rel| {
            let abs = base.join(rel);
            let project = project::find(abs.parent().unwrap_or(base)).filter(|m| m.starts_with(base)).map(|m| {
                projects
                    .entry(m.clone())
                    .or_insert_with(|| {
                        let p = project::load(&m).ok();
                        let name = p.as_ref().map(|p| p.manifest.project.name.clone()).unwrap_or_else(|| "?".into());
                        let entries = p.as_ref().map(|p| entry_points(&p.manifest)).unwrap_or_default();
                        let dir = m.parent().and_then(|d| d.strip_prefix(base).ok()).map(slash).unwrap_or_default();
                        json!({ "name": name, "dir": dir, "entries": entries })
                    })
                    .clone()
            });
            let text = std::fs::read_to_string(&abs).unwrap_or_default();
            let (mut items, error) = match vandorf::parser::parse(&text, 0) {
                Ok(file) => (outline(&file, &text), None),
                Err(errs) => (vec![], errs.first().map(|e| e.to_string())),
            };
            // Top-level procs a bin or a route runs: what runs them.
            if let Some(entries) = project.as_ref().and_then(|p| p["entries"].as_object()) {
                for it in items.iter_mut().filter(|it| it["kind"] == "proc") {
                    if let Some(by) = it["name"].as_str().and_then(|n| entries.get(n)) {
                        it["entry"] = by.clone();
                    }
                }
            }
            json!({ "path": rel, "project": project, "items": items, "error": error })
        })
        .collect();
    Ok(json!({ "ok": true, "root": base.display().to_string(), "files": out }))
}

/// The project's entry points by proc: `bin demo`, `POST /api/calc`, and
/// `GET / (raw)` for a route whose proc takes and returns HTTP.
fn entry_points(m: &project::Manifest) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, b) in &m.bin {
        out.entry(b.entry.clone()).or_default().push(format!("bin {name}"));
    }
    for s in m.http.values() {
        for (route, proc) in &s.routes {
            out.entry(proc.clone()).or_default().push(route.clone());
        }
        for (route, proc) in &s.raw {
            out.entry(proc.clone()).or_default().push(format!("{route} (raw)"));
        }
    }
    out
}

/// A project's overview: its manifest's settings, entry points (each with
/// where its proc is declared), and counts.
fn project_info(base: &Path, dir: &str) -> Result<Value, String> {
    let root = if dir.is_empty() || dir == "." { base.to_path_buf() } else { resolve(base, dir)? };
    let manifest = root.join(project::MANIFEST);
    if !manifest.is_file() {
        return Err(format!("no {} in {dir}", project::MANIFEST));
    }
    let text = std::fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
    let p = project::load(&manifest)?;
    let m = &p.manifest;
    let rel = |path: &Path| path.strip_prefix(base).map(slash).unwrap_or_else(|_| slash(path));
    // Top-level procs, and counts, from the outlines of the project's files.
    let mut procs: HashMap<String, Value> = HashMap::new();
    let (mut nproc, mut nfunc, mut ntype) = (0, 0, 0);
    fn count(items: &[Value], nproc: &mut usize, nfunc: &mut usize, ntype: &mut usize) {
        for it in items {
            match it["kind"].as_str() {
                Some("proc") => *nproc += 1,
                Some("func") => *nfunc += 1,
                Some("type") => *ntype += 1,
                _ => {}
            }
            if let Some(k) = it["children"].as_array() {
                count(k, nproc, nfunc, ntype);
            }
        }
    }
    let files: Vec<String> = p.files.iter().map(|f| rel(&p.root.join(&f.path))).collect();
    let mut errors = vec![];
    for (f, path) in p.files.iter().zip(&files) {
        match vandorf::parser::parse(&f.text, 0) {
            Ok(file) => {
                let items = outline(&file, &f.text);
                count(&items, &mut nproc, &mut nfunc, &mut ntype);
                for it in items.iter().filter(|it| it["kind"] == "proc") {
                    if let Some(n) = it["name"].as_str() {
                        procs.insert(n.to_string(), json!({ "path": path, "line": it["line"] }));
                    }
                }
            }
            Err(_) => errors.push(path.clone()),
        }
    }
    let at = |proc: &str| procs.get(proc).cloned().unwrap_or(Value::Null);
    let bins: Vec<Value> = m
        .bin
        .iter()
        .map(|(name, b)| {
            json!({
                "name": name,
                "entry": b.entry,
                "at": at(&b.entry),
                "go": m.go.as_ref().map(|g| {
                    let d = b.go.clone().unwrap_or_else(|| format!("{}/../cmd/{name}", g.out));
                    project::slash(&project::relative(&project::normalize(&p.root.join(d)), &p.root))
                }),
                "rust": m.rust.as_ref().map(|_| b.rust.clone().unwrap_or_else(|| format!("bin/{name}.rs"))),
            })
        })
        .collect();
    let route = |r: &str, proc: &str, raw: bool| {
        let (method, path) = r.split_once(' ').unwrap_or(("", r));
        json!({ "method": method, "path": path, "proc": proc, "raw": raw, "at": at(proc) })
    };
    let servers: Vec<Value> = m
        .http
        .iter()
        .map(|(name, s)| {
            let mut routes: Vec<Value> = s.routes.iter().map(|(r, p)| route(r, p, false)).collect();
            routes.extend(s.raw.iter().map(|(r, p)| route(r, p, true)));
            json!({
                "name": name,
                "addr": s.addr,
                "rust": s.rust.clone().unwrap_or_else(|| format!("bin/{name}.rs")),
                "routes": routes,
                "statics": s.statics.iter().map(|(u, f)| json!({ "path": u, "file": f })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({
        "ok": true,
        "name": m.project.name,
        "dir": rel(&p.root),
        "manifest": rel(&manifest),
        "text": text,
        "sources": m.project.sources,
        "go": m.go.as_ref().map(|g| json!({ "out": g.out, "package": g.package, "module": g.module, "types_dir": g.types_dir })),
        "rust": m.rust.as_ref().map(|r| json!({ "out": r.out, "module_dir": r.module_dir })),
        "bins": bins,
        "servers": servers,
        "files": files,
        "unparsed": errors,
        "counts": { "files": p.files.len(), "procs": nproc, "funcs": nfunc, "types": ntype },
    }))
}

/// Declarations of one file, procs with their nested procs and funcs.
/// `qual` is the name the document's grids and the symbols use.
fn outline(file: &vandorf::ast::File, src: &str) -> Vec<Value> {
    fn node(n: &vandorf::ast::Node, prefix: &str, src: &str) -> Value {
        match n {
            vandorf::ast::Node::Func(f) => item("func", &f.name, prefix, src, vec![]),
            vandorf::ast::Node::Proc(p) => {
                let qual = qualify(prefix, &p.name.name);
                let kids = p.nodes.iter().map(|k| node(k, &qual, src)).collect();
                item("proc", &p.name, prefix, src, kids)
            }
        }
    }
    let mut out: Vec<Value> = vec![];
    for t in &file.types {
        out.push(item("type", &t.name, "", src, vec![]));
    }
    for i in &file.interfaces {
        let kids = i.funcs.iter().map(|f| item("func", &f.name, &i.name.name, src, vec![])).collect();
        out.push(item("interface", &i.name, "", src, kids));
    }
    for n in &file.nodes {
        out.push(node(n, "", src));
    }
    out.sort_by_key(|v| v["line"].as_u64());
    out
}

fn qualify(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_string() } else { format!("{prefix}.{name}") }
}

fn item(kind: &str, name: &vandorf::ast::Ident, prefix: &str, src: &str, children: Vec<Value>) -> Value {
    json!({
        "kind": kind,
        "name": name.name,
        "qual": qualify(prefix, &name.name),
        "line": line_col(src, name.span.start)["line"],
        "children": children,
    })
}

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/* ---------- saving ---------- */

fn save(base: &Path, body: &str) -> Result<Value, String> {
    let req: Value = serde_json::from_str(body).map_err(|e| format!("bad request: {e}"))?;
    let p = req["path"].as_str().ok_or("missing path")?;
    let text = req["source"].as_str().ok_or("missing source")?;
    let abs = resolve(base, p)?;
    let manifest = abs.file_name().is_some_and(|n| n == project::MANIFEST);
    if !(manifest || abs.extension().is_some_and(|x| x.eq_ignore_ascii_case("sys"))) || !abs.is_file() {
        return Err(format!("only existing .sys files and manifests can be saved: {p}"));
    }
    // A manifest that doesn't parse would break the project (and this
    // page's view of it), so it isn't written.
    if manifest {
        toml::from_str::<project::Manifest>(text).map_err(|e| format!("invalid manifest, not saved: {}", e.message()))?;
    }
    std::fs::write(&abs, text).map_err(|e| format!("cannot write {p}: {e}"))?;
    Ok(json!({ "ok": true, "path": p, "bytes": text.len() }))
}

/* ---------- symbols, for the viewer's preview drawer ---------- */

/// Typedefs, funcs and procs of a checked program, with where they're
/// declared (paths relative to the source dir, 0-based lines) and, for
/// funcs, their implementation files resolved the same way.
fn symbols(base: &Path, prog: &Program, files: &[vandorf::Source], roots: &[PathBuf]) -> Value {
    let rel = |p: &Path| p.strip_prefix(base).map(slash).ok();
    let types: serde_json::Map<String, Value> =
        prog.types.iter().map(|t| (t.name.clone(), json!({ "def": t.ty.to_string(), "recursive": t.recursive }))).collect();
    let mut nodes = serde_json::Map::new();
    for n in &prog.nodes {
        let fid = n.span.context;
        let (Some(f), Some(root)) = (files.get(fid), roots.get(fid)) else { continue };
        let abs = project::normalize(&root.join(&f.path));
        let loc = json!({ "file": rel(&abs), "line": line_col(&f.text, n.span.start)["line"] });
        let params = |ps: &[(String, vandorf::types::Type)]| -> String {
            ps.iter().map(|(p, t)| format!("{p}: {t}")).collect::<Vec<_>>().join(", ")
        };
        let v = match &n.kind {
            NodeKind::Func(func) => {
                let dir = abs.parent().unwrap_or(base);
                let imps: Vec<Value> = func
                    .imps
                    .iter()
                    .map(|i| {
                        let p = project::normalize(&dir.join(&i.written));
                        json!({
                            "written": i.written,
                            "path": rel(&p),
                            "function": i.function,
                            "lang": i.written.rsplit_once('.').map(|(_, e)| e).unwrap_or(""),
                        })
                    })
                    .collect();
                json!({ "kind": "func", "params": params(&func.params), "ret": func.ret.to_string(), "imps": imps, "loc": loc })
            }
            NodeKind::Proc(p) => json!({
                "kind": "proc",
                "params": params(prog.params(p.entry)),
                "ret": p.ret.to_string(),
                "entry": prog.node(p.entry).name(),
                "loc": loc,
            }),
            _ => continue,
        };
        nodes.insert(n.qual_name(), v);
    }
    json!({ "types": types, "nodes": nodes })
}

/* ---------- terminals ---------- */

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
    /// Stream offset of `data[0]`.
    start: u64,
    closed: bool,
}

impl Term {
    fn spawn(cwd: &Path) -> Result<Arc<Term>, String> {
        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/bash".into());
        // script(1) runs the shell on a pty, so line editing, colors and
        // job control work although we talk to it over pipes.
        let mut child = Command::new("script")
            .args(["-qfec", &format!("{shell} -i"), "/dev/null"])
            .current_dir(cwd)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env("PATH", path_with_self())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start a shell with script(1): {e}"))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let t = Arc::new(Term {
            pid: child.id(),
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            out: Mutex::new(TermOut::default()),
            ready: Condvar::new(),
        });
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
        Ok(t)
    }

    /// Output from stream offset `from`, waiting up to `wait` for some.
    fn read(&self, from: u64, wait: Duration) -> (Vec<u8>, u64, bool) {
        let mut o = self.out.lock().unwrap();
        let end = |o: &TermOut| o.start + o.data.len() as u64;
        if end(&o) <= from && !o.closed {
            o = self.ready.wait_timeout_while(o, wait, |o| end(o) <= from && !o.closed).unwrap().0;
        }
        let from = from.clamp(o.start, end(&o));
        (o.data[(from - o.start) as usize..].to_vec(), end(&o), o.closed)
    }

    /// The pty's slave device: the shell's stdin, a child of script(1).
    fn pty(&self) -> Option<PathBuf> {
        let kids = std::fs::read_to_string(format!("/proc/{0}/task/{0}/children", self.pid)).ok()?;
        let kid = kids.split_whitespace().next()?;
        std::fs::read_link(format!("/proc/{kid}/fd/0")).ok().filter(|p| p.starts_with("/dev/pts"))
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let pty = self.pty().ok_or("terminal has no pty yet")?;
        // Setting the size on the slave signals SIGWINCH to its foreground job.
        let ok = Command::new("stty")
            .arg("-F")
            .arg(&pty)
            .args(["rows", &rows.to_string(), "cols", &cols.to_string()])
            .status()
            .map_err(|e| format!("stty: {e}"))?
            .success();
        if ok { Ok(()) } else { Err("stty failed".into()) }
    }
}

/// `PATH` with this executable's directory first, so `vandorf` in the
/// terminal is the one serving.
fn path_with_self() -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).into_iter().collect();
    dirs.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
    std::env::join_paths(dirs).unwrap_or_default()
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.child.get_mut().map(|c| c.kill());
    }
}

fn term_api(req: &mut Request, st: &State, path: &str, query: &str) -> tiny_http::ResponseBox {
    let id: Option<u32> = query_param(query, "id").and_then(|s| s.parse().ok());
    let get = || id.and_then(|id| st.terms.lock().unwrap().get(&id).cloned()).ok_or_else(|| "no such terminal".to_string());
    match (req.method(), path) {
        (Method::Post, "/api/term") => {
            let cwd = query_param(query, "cwd").and_then(|c| resolve(&st.base, &c).ok()).filter(|p| p.is_dir());
            api(Term::spawn(cwd.as_deref().unwrap_or(&st.base)).map(|t| {
                let id = st.next_term.fetch_add(1, Ordering::Relaxed);
                st.terms.lock().unwrap().insert(id, t);
                json!({ "ok": true, "id": id })
            }))
        }
        (Method::Get, "/api/term/list") => {
            let ids: Vec<u32> = st.terms.lock().unwrap().keys().copied().collect();
            api(Ok(json!({ "ok": true, "ids": ids })))
        }
        (Method::Get, "/api/term/read") => match get() {
            Ok(t) => {
                let from = query_param(query, "from").and_then(|s| s.parse().ok()).unwrap_or(0);
                let (data, next, closed) = t.read(from, Duration::from_secs(20));
                Response::from_data(data)
                    .with_header(header("application/octet-stream"))
                    .with_header(Header::from_bytes("X-Next", next.to_string()).unwrap())
                    .with_header(Header::from_bytes("X-Closed", if closed { "1" } else { "0" }).unwrap())
                    .boxed()
            }
            Err(e) => api(Err(e)),
        },
        (Method::Post, "/api/term/input") => {
            let mut data = vec![];
            let read = req.as_reader().take(1 << 20).read_to_end(&mut data);
            api(read.map_err(|e| e.to_string()).and_then(|_| get()).and_then(|t| {
                let mut w = t.stdin.lock().unwrap();
                w.write_all(&data).and_then(|_| w.flush()).map_err(|e| format!("terminal closed: {e}"))?;
                Ok(json!({ "ok": true }))
            }))
        }
        (Method::Post, "/api/term/resize") => {
            let n = |k| query_param(query, k).and_then(|s| s.parse::<u16>().ok()).filter(|&n| n > 0);
            api(get().and_then(|t| match (n("cols"), n("rows")) {
                (Some(c), Some(r)) => t.resize(c, r).map(|_| json!({ "ok": true })),
                _ => Err("need cols and rows".into()),
            }))
        }
        (Method::Post, "/api/term/kill") => api(get().map(|t| {
            let _ = t.child.lock().unwrap().kill();
            st.terms.lock().unwrap().remove(&id.unwrap());
            json!({ "ok": true })
        })),
        _ => Response::from_string("not found").with_status_code(404).boxed(),
    }
}

/// Compiles the editor's text. If the file belongs to a project (a
/// `vandorf.toml` at or above it, inside the source directory), the whole
/// project is compiled with the editor's text in place of the file, and
/// previewed with the manifest's settings; otherwise just the text, with
/// nominal output locations. Nothing is written.
fn compile(base: &Path, body: &str) -> Result<Value, String> {
    let req: Value = serde_json::from_str(body).map_err(|e| format!("bad request: {e}"))?;
    let path = req["path"].as_str().filter(|p| !p.is_empty());
    let text = match (req["source"].as_str(), path) {
        (Some(s), _) if !s.trim().is_empty() => s.to_string(),
        (_, Some(p)) => read_source(base, p)?,
        _ => return Err(r#"provide either "path" or "source""#.into()),
    };
    let abs = path.map(|p| resolve(base, p)).transpose()?;
    let manifest = abs.as_ref().and_then(|a| project::find(a.parent()?)).filter(|m| m.starts_with(base));

    let (files, dirs, proj, here) = match manifest {
        Some(m) => {
            let mut p = project::load(&m)?;
            let abs = abs.unwrap();
            if !p.set_text(&abs, text.clone()) {
                return Err(format!("{} is not under the sources of {}", abs.display(), m.display()));
            }
            let here = p.files.iter().position(|f| project::normalize(&p.root.join(&f.path)) == project::normalize(&abs)).unwrap();
            (p.files.clone(), p.dirs(), Some(p), here)
        }
        None => {
            let name = path.map(str::to_string).or(req["name"].as_str().map(str::to_string)).unwrap_or_else(|| "<editor>".into());
            let dir = abs.as_ref().and_then(|a| a.parent()).unwrap_or(base).to_path_buf();
            (vec![vandorf::Source { path: name, text }], vec![dir], None, 0)
        }
    };

    let mut prog = match vandorf::check_files(&files) {
        Ok(p) => p,
        Err(diags) => {
            // 200 with ok:false, so the client can still read the diagnostics.
            let list: Vec<Value> = diags
                .iter()
                .map(|d| {
                    let src = &files[d.file].text;
                    json!({
                        "msg": d.msg,
                        "file": files[d.file].path,
                        "here": d.file == here,
                        "from": line_col(src, d.span.start),
                        "to": line_col(src, d.span.end),
                    })
                })
                .collect();
            return Ok(json!({ "ok": false, "error": vandorf::render(&files, &diags, false), "diagnostics": list }));
        }
    };

    let doc = vandorf::document::generate(&prog);
    // Before previewing, which rebases impl paths onto the outputs.
    let roots: Vec<PathBuf> = match &proj {
        Some(p) => vec![p.root.clone(); files.len()],
        None => vec![base.to_path_buf(); files.len()],
    };
    let syms = symbols(base, &prog, &files, &roots);
    let (go, rust) = match &proj {
        Some(p) => preview_project(p, &mut prog, &dirs),
        None => {
            // Nominal locations: impl paths stay as written, as if the
            // output were next to the file.
            let main = codegen::has_proc(&prog, codegen::MAIN_PROC).then(|| MainSpec {
                name: "main".into(),
                entry: codegen::MAIN_PROC.into(),
                path: String::new(),
            });
            let mut go_opts = GoOptions::new("app", Some("example.com/app".into()));
            go_opts.mains = main.iter().map(|m| MainSpec { path: "main".into(), ..m.clone() }).collect();
            let rust_opts = RustOptions {
                module_dir: Some("app".into()),
                mains: main.iter().map(|m| MainSpec { path: "main.rs".into(), ..m.clone() }).collect(),
                crate_name: Some("my_crate".into()),
                servers: vec![],
            };
            (Some(codegen::go::generate(&prog, &go_opts)), Some(codegen::rust::generate(&prog, &rust_opts)))
        }
    };
    let none = || Err("not configured in vandorf.toml".to_string());
    Ok(json!({
        "ok": true,
        "name": files[here].path,
        "project": proj.as_ref().map(|p| p.manifest.project.name.clone()),
        "grids": doc.grids.len(),
        "document": doc,
        "symbols": syms,
        "go": output_json(go.unwrap_or_else(none)),
        "rust": output_json(rust.unwrap_or_else(none)),
    }))
}

/// A backend's output, if the project configures it.
type Preview = Option<Result<Output, String>>;

/// What `vandorf build` would generate for a project, per backend.
fn preview_project(p: &project::Project, prog: &mut vandorf::sema::Program, dirs: &[PathBuf]) -> (Preview, Preview) {
    let m = &p.manifest;
    let go = m.go.as_ref().map(|g| {
        let out = p.root.join(&g.out);
        project::rebase_impls(prog, dirs, &out);
        let package = g.package.clone().unwrap_or_else(|| out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or("app".into()));
        let mut opts = GoOptions::new(package, g.module.clone().or_else(|| crate::go_import_path(&out)));
        if let Some(d) = &g.types_dir {
            opts.types_dir = d.clone();
        }
        opts.mains = crate::go_mains(p, g, &m.bin.iter().collect::<Vec<_>>());
        codegen::go::generate(prog, &opts)
    });
    let rust = m.rust.as_ref().map(|r| {
        let out = p.root.join(&r.out);
        project::rebase_impls(prog, dirs, &out);
        let opts = RustOptions {
            module_dir: r.module_dir.clone().or_else(|| crate::rust_module_dir(&out)),
            mains: crate::rust_mains(&m.bin.iter().collect::<Vec<_>>()),
            crate_name: crate::rust_lib_name(&out),
            servers: vec![],
        };
        codegen::rust::generate(prog, &opts)
    });
    (go, rust)
}

fn output_json(r: Result<Output, String>) -> Value {
    match r {
        Ok(out) => {
            let artifacts: Vec<Value> = [("types", out.types), ("code", out.code)]
                .into_iter()
                .chain(out.mains.into_iter().map(|m| ("main", m)))
                .map(|(kind, a)| json!({ "kind": kind, "dir": a.dir, "files": a.files }))
                .collect();
            json!({
            "ok": true,
            "artifacts": artifacts,
            "impls": out.impls.iter().map(|i| json!({
                "node": i.node, "file": i.file, "function": i.function, "signature": i.signature,
            })).collect::<Vec<_>>(),
            })
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// 0-based line and UTF-16 column of a byte offset, as CodeMirror expects.
fn line_col(src: &str, offset: usize) -> Value {
    let offset = offset.min(src.len());
    let before = &src[..src.floor_char_boundary(offset)];
    let line = before.matches('\n').count();
    let col = before.rsplit('\n').next().unwrap_or("").encode_utf16().count();
    json!({ "line": line, "ch": col })
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolves `p` under `base`, refusing paths that escape it.
fn resolve(base: &Path, p: &str) -> Result<PathBuf, String> {
    if p.is_empty() || p.contains('\0') {
        return Err("invalid path".into());
    }
    let full = std::fs::canonicalize(base.join(p)).map_err(|e| format!("cannot read {p}: {e}"))?;
    if !full.starts_with(base) {
        return Err(format!("path escapes source directory: {p}"));
    }
    Ok(full)
}

fn read_source(base: &Path, p: &str) -> Result<String, String> {
    std::fs::read_to_string(resolve(base, p)?).map_err(|e| format!("cannot read {p}: {e}"))
}

fn list_sources(base: &Path) -> Result<Vec<String>, String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !name.starts_with('.') && name != "target" && name != "node_modules" {
                    walk(&p, base, out);
                }
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("sys"))
                && let Ok(rel) = p.strip_prefix(base)
            {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = vec![];
    walk(base, base, &mut out);
    out.sort();
    Ok(out)
}
