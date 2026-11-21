//! `vandorf serve`: a local workbench (editor + visualizer + Go preview).
//!
//! Routes:
//! - `GET /`              workbench page
//! - `GET /viz`           the visualizer (also usable standalone)
//! - `GET /api/sources`   `.sys` files under the source dir
//! - `GET /api/source?path=..`  one file's text
//! - `POST /api/compile`  `{source}` or `{path}` -> document + generated Go,
//!   or diagnostics. Same response shape as the Go visualizer server.

use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use tiny_http::{Header, Method, Request, Response, Server};
use vandorf::codegen::go::GoOptions;
use vandorf::codegen::rust::RustOptions;
use vandorf::codegen::{self, MainSpec, Output};
use vandorf::project;

const WORKBENCH: &str = include_str!("../web/workbench.html");
const VISUALIZER: &str = include_str!("../web/visualizer.html");

pub fn serve(addr: &str, dir: &Path) -> Result<(), String> {
    let base = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let server = Server::http(addr).map_err(|e| format!("cannot listen on {addr}: {e}"))?;
    eprintln!("vandorf workbench on http://{addr}  (sources: {})", base.display());
    for req in server.incoming_requests() {
        handle(req, &base);
    }
    Ok(())
}

fn handle(mut req: Request, base: &Path) {
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let resp = match (req.method(), path) {
        (Method::Get, "/") => html(WORKBENCH),
        (Method::Get, "/viz" | "/viz/") => html(VISUALIZER),
        (Method::Get, "/api/sources") => api(list_sources(base).map(|files| {
            json!({ "ok": true, "root": base.display().to_string(), "files": files })
        })),
        (Method::Get, "/api/source") => api(query_param(query, "path")
            .ok_or_else(|| "missing ?path=".to_string())
            .and_then(|p| read_source(base, &p).map(|src| json!({ "ok": true, "path": p, "source": src })))),
        (Method::Post, "/api/compile") => {
            let mut body = String::new();
            let read = req.as_reader().take(1 << 20).read_to_string(&mut body);
            api(read.map_err(|e| format!("bad request: {e}")).and_then(|_| compile(base, &body)))
        }
        _ => Response::from_string("not found").with_status_code(404).boxed(),
    };
    let _ = req.respond(resp);
}

fn html(s: &'static str) -> tiny_http::ResponseBox {
    Response::from_string(s).with_header(header("text/html; charset=utf-8")).boxed()
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
                servers: vec![],
                module_dir: Some("app".into()),
                mains: main.iter().map(|m| MainSpec { path: "main.rs".into(), ..m.clone() }).collect(),
                crate_name: Some("my_crate".into()),
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
            servers: vec![],
            module_dir: r.module_dir.clone().or_else(|| crate::rust_module_dir(&out)),
            mains: crate::rust_mains(&m.bin.iter().collect::<Vec<_>>()),
            crate_name: crate::rust_lib_name(&out),
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
