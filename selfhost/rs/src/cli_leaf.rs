//! Leaves for vcc's command line (../sys/cli): the manifest, paths, and
//! the Rust main file.

use crate::vcc::types::*;
use std::path::{Component, Path, PathBuf};

fn s(x: &str) -> Str {
    x.into()
}

fn opt(v: Option<&toml::Value>) -> MaybeStr {
    v.and_then(|v| v.as_str()).map_or(MaybeStr::None, |x| MaybeStr::Some(s(x)))
}

/// vandorf.toml, as src/project.rs reads it.
pub fn parse_manifest(text: Str) -> Result<ResManifest, String> {
    let fail = |e: String| Ok(ResManifest::Err(Diag { msg: e.into(), span: Span { at: 0, end: 0, file: 0 } }));
    let t: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => return fail(e.to_string()),
    };
    let Some(project) = t.get("project").and_then(|p| p.as_table()) else { return fail("missing field `project`".into()) };
    let Some(name) = project.get("name").and_then(|n| n.as_str()) else { return fail("missing field `name`".into()) };
    let sources: Vec<Str> = match project.get("sources").and_then(|v| v.as_array()) {
        Some(a) => a.iter().filter_map(|x| x.as_str()).map(s).collect(),
        None => vec![s(".")],
    };
    let rust = match t.get("rust").and_then(|r| r.as_table()) {
        Some(r) => match r.get("out").and_then(|o| o.as_str()) {
            Some(out) => MaybeRust::Some(RustTarget { out: s(out), module_dir: opt(r.get("module_dir")) }),
            None => return fail("[rust]: missing field `out`".into()),
        },
        None => MaybeRust::None,
    };
    let mut http = vec![];
    if let Some(h) = t.get("http").and_then(|h| h.as_table()) {
        for (n, v) in h {
            let Some(v) = v.as_table() else { continue };
            let Some(addr) = v.get("addr").and_then(|a| a.as_str()) else { return fail(format!("[http.{n}]: missing field `addr`")) };
            let Some(routes) = v.get("routes").and_then(|r| r.as_table()) else { return fail(format!("[http.{n}]: missing field `routes`")) };
            let strs = |t: &toml::Table| -> Map<Str, Str> { t.iter().filter_map(|(k, v)| v.as_str().map(|v| (s(k), s(v)))).collect() };
            let statics = v.get("static").and_then(|x| x.as_table()).map(strs).unwrap_or_default();
            http.push((s(n), ServerDecl { addr: s(addr), routes: strs(routes), statics, rust: opt(v.get("rust")) }));
        }
    }
    let mut bins = vec![];
    if let Some(b) = t.get("bin").and_then(|b| b.as_table()) {
        for (n, v) in b {
            let Some(v) = v.as_table() else { continue };
            bins.push((s(n), Bin { entry: v.get("entry").and_then(|e| e.as_str()).map_or(s("Main"), s), go: opt(v.get("go")), rust: opt(v.get("rust")) }));
        }
    }
    Ok(ResManifest::Ok(Manifest { name: s(name), sources: sources.into(), rust, has_go: t.contains_key("go"), bins: bins.into_iter().collect(), http: http.into_iter().collect() }))
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// a/b, `.` and `..` resolved lexically (b may be absolute).
pub fn join(a: Str, b: Str) -> Result<Str, String> {
    Ok(slash(&normalize(&Path::new(&*a).join(&*b))).into())
}

pub fn parent(p: Str) -> Result<Str, String> {
    Ok(Path::new(&*p).parent().map_or(s(""), |d| slash(d).into()))
}

/// target relative to the directory from; both absolute and normalized.
pub fn relative(target: Str, from: Str) -> Result<Str, String> {
    let (t, f): (Vec<_>, Vec<_>) = (Path::new(&*target).components().collect(), Path::new(&*from).components().collect());
    let common = t.iter().zip(&f).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..f.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c);
    }
    Ok(slash(&out).into())
}

pub fn sort_dedup(xs: List<Str>) -> Result<List<Str>, String> {
    let mut v: Vec<Str> = xs.to_vec();
    v.sort();
    v.dedup();
    Ok(v.into())
}

pub fn starts_with(text: Str, prefix: Str) -> Result<bool, String> {
    Ok(text.starts_with(&*prefix))
}

/// Each implementation path made relative to `out` instead of the .sys
/// file naming it (dirs[file id] is that file's directory).
pub fn rebase_impls(prog: Program, dirs: List<Str>, out: Str) -> Result<Program, String> {
    let mut prog = prog;
    let nodes: Vec<NodeDecl> = prog
        .nodes
        .iter()
        .map(|n| {
            let mut n = n.clone();
            if let NodeKind::Func(f) = &n.kind {
                let mut f = f.clone();
                f.imps = f
                    .imps
                    .iter()
                    .map(|i| {
                        let mut i = i.clone();
                        let full = normalize(&Path::new(&*dirs[i.span.file as usize]).join(&*i.file));
                        i.file = relative(slash(&full).into(), out.clone()).unwrap();
                        i
                    })
                    .collect();
                n.kind = NodeKind::Func(f);
            }
            n
        })
        .collect();
    prog.nodes = nodes.into();
    Ok(prog)
}

/// `name = "..."` in [lib], else in [package], of a Cargo.toml (with
/// `-` as `_`); "" if the crate has no library.
pub fn cargo_lib_name(toml_text: Str, has_lib_rs: bool) -> Result<Str, String> {
    let mut section = "";
    let (mut lib, mut package) = (None, None);
    for l in toml_text.lines().map(str::trim) {
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
    let has_lib = has_lib_rs || toml_text.lines().any(|l| l.trim() == "[lib]");
    Ok(if has_lib { lib.or(package).unwrap_or_default().into() } else { s("") })
}

/// The path segments from root down to dir.
pub fn rel_segs(root: Str, dir: Str) -> Result<List<Str>, String> {
    Ok(Path::new(&*dir)
        .strip_prefix(&*root)
        .map(|r| r.components().map(|c| Str::from(c.as_os_str().to_string_lossy().as_ref())).collect())
        .unwrap_or_default())
}

pub fn unwrap_m(r: ResManifest) -> Result<Manifest, String> {
    match r {
        ResManifest::Ok(m) => Ok(m),
        ResManifest::Err(e) => Err(e.msg.to_string()),
    }
}

pub fn unwrap_paths(r: ResStringList) -> Result<List<Str>, String> {
    match r {
        ResStringList::Ok(p) => Ok(p),
        ResStringList::Err(e) => Err(e.msg.to_string()),
    }
}

pub fn rel_all(paths: List<Str>, root: Str) -> Result<List<Str>, String> {
    Ok(paths.iter().map(|p| Path::new(&**p).strip_prefix(&*root).map_or(p.clone(), |r| slash(r).into())).collect())
}

pub fn unwrap_p(r: ResProject) -> Result<Project, String> {
    match r {
        ResProject::Ok(p) => Ok(p),
        ResProject::Err(e) => Err(e.msg.to_string()),
    }
}

pub fn unwrap_r(r: MaybeRust) -> Result<RustTarget, String> {
    match r {
        MaybeRust::Some(t) => Ok(t),
        MaybeRust::None => Err("no [rust]".into()),
    }
}

pub fn render_errors(errors: List<Diag>, paths: List<Str>) -> Result<Str, String> {
    Ok(errors
        .iter()
        .map(|e| format!("{}:{}..{}: {}", paths.get(e.span.file as usize).map_or("?", |p| &**p), e.span.at, e.span.end, e.msg))
        .collect::<Vec<_>>()
        .join("\n")
        .into())
}

pub fn out_dir(c: Checked) -> Result<Str, String> {
    let r = unwrap_r(c.project.manifest.rust)?;
    join(c.project.root, r.out)
}

pub fn out_paths(files: List<OutFile>) -> Result<List<Str>, String> {
    Ok(files.iter().map(|f| f.path.clone()).collect())
}

pub fn dirs(root: Str, paths: List<Str>) -> Result<List<Str>, String> {
    paths.iter().map(|p| parent(join(root.clone(), p.clone())?)).collect::<Result<Vec<_>, _>>().map(List::from)
}

pub fn rebased(c: Checked) -> Result<Program, String> {
    let d = dirs(c.project.root.clone(), c.project.paths.clone())?;
    rebase_impls(c.prog.clone(), d, out_dir(c)?)
}

pub fn module_dir(target: RustTarget, out: Str, croot: Str) -> Result<MaybeStr, String> {
    if let MaybeStr::Some(d) = target.module_dir {
        return Ok(MaybeStr::Some(d));
    }
    if croot.is_empty() {
        return Ok(MaybeStr::None);
    }
    let segs = rel_segs(croot, out)?;
    Ok(match segs.split_first() {
        Some((src, rest)) if &**src == "src" => MaybeStr::Some(rest.iter().map(|s| &**s).collect::<Vec<_>>().join("/").into()),
        _ => MaybeStr::None,
    })
}

pub fn bin_names(bins: Map<Str, Bin>, only: Str) -> Result<List<Str>, String> {
    if only.is_empty() {
        return Ok(bins.keys().cloned().collect());
    }
    if bins.contains_key(&only) {
        return Ok(vec![only].into());
    }
    let known: Vec<&str> = bins.keys().map(|k| &**k).collect();
    Err(format!("vandorf.toml: no [bin.{only}]; bins: {}", if known.is_empty() { "none".into() } else { known.join(", ") }))
}

pub fn bin_path(b: Bin, name: Str) -> Result<Str, String> {
    Ok(match b.rust {
        MaybeStr::Some(p) => p,
        MaybeStr::None => format!("bin/{name}.rs").into(),
    })
}

pub fn refuse(msg: Str) -> Result<bool, String> {
    Err(msg.to_string())
}

pub fn pad(s: Str, n: i64) -> Result<Str, String> {
    Ok(format!("{s:<width$}", width = n as usize).into())
}

pub fn fail_files(msg: Str) -> Result<List<OutFile>, String> {
    Err(msg.to_string())
}

pub fn no_specs(_n: i64) -> Result<ResServerSpecList, String> {
    Ok(ResServerSpecList::Ok(List::default()))
}

pub fn unwrap_specs(r: ResServerSpecList) -> Result<List<ServerSpec>, String> {
    match r {
        ResServerSpecList::Ok(x) => Ok(x),
        ResServerSpecList::Err(e) => Err(e.msg.to_string()),
    }
}

pub fn unwrap_routes(r: ResRouteList) -> Result<List<Route>, String> {
    match r {
        ResRouteList::Ok(x) => Ok(x),
        ResRouteList::Err(e) => Err(e.msg.to_string()),
    }
}

pub fn http_file(out: Str, text: Str) -> Result<List<OutFile>, String> {
    Ok(if text.is_empty() { List::default() } else { vec![OutFile { path: join(out, "http.rs".into())?, text, kind: "code".into() }].into() })
}

pub fn server_path(d: ServerDecl, name: Str) -> Result<Str, String> {
    Ok(match d.rust {
        MaybeStr::Some(p) => p,
        MaybeStr::None => format!("bin/{name}.rs").into(),
    })
}

pub fn file_dir(out: Str, module_dir: MaybeStr, path: Str) -> Result<Str, String> {
    let up = match &module_dir {
        MaybeStr::Some(d) => d.split('/').filter(|x| !x.is_empty()).count(),
        MaybeStr::None => 0,
    };
    let mut src = normalize(Path::new(&*out));
    for _ in 0..up {
        src.pop();
    }
    let full = normalize(&src.join(&*path));
    Ok(slash(full.parent().unwrap_or(&src)).into())
}

pub fn split_route(route: Str, proc: Str) -> Result<List<Route>, String> {
    let Some((m, p)) = route.split_once(' ').map(|(m, p)| (m.trim(), p.trim())) else { return Ok(List::default()) };
    if m.is_empty() || !m.chars().all(|c| c.is_ascii_uppercase()) || !p.starts_with('/') {
        return Ok(List::default());
    }
    Ok(vec![Route { method: m.into(), path: p.into(), proc }].into())
}

pub fn debug_quote(s: Str) -> Result<Str, String> {
    Ok(format!("{:?}", &*s).into())
}

pub fn content_type(file: Str) -> Result<Str, String> {
    let ext = Path::new(&*file).extension().and_then(|e| e.to_str()).unwrap_or("");
    Ok(match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        _ => "text/plain; charset=utf-8",
    }
    .into())
}
