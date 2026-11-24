//! Leaves for vcc's command line (the .sys files here): the manifest, paths, and
//! the Rust main file.

use crate::vcc::types::*;
use std::path::{Component, Path, PathBuf};

fn s(x: &str) -> Str {
    x.into()
}

fn opt(v: Option<&toml::Value>) -> MaybeStr {
    v.and_then(|v| v.as_str())
        .map_or(MaybeStr::None, |x| MaybeStr::Some(s(x)))
}

/// vandorf.toml, as src/project.rs reads it.
pub fn parse_manifest(text: Str) -> Result<ResManifest, String> {
    let fail = |e: String| {
        Ok(ResManifest::Err(Diag {
            msg: e.into(),
            span: Span {
                at: 0,
                end: 0,
                file: 0,
            },
        }))
    };
    let t: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => return fail(e.to_string()),
    };
    // Refuse what the reference refuses, rather than ignore it.
    if let Err(e) = check_table(&t, MANIFEST) {
        return fail(e);
    }
    let Some(project) = t.get("project").and_then(|p| p.as_table()) else {
        return fail("missing field `project`".into());
    };
    let Some(name) = project.get("name").and_then(|n| n.as_str()) else {
        return fail("missing field `name`".into());
    };
    let sources: Vec<Str> = match project.get("sources").and_then(|v| v.as_array()) {
        Some(a) => a.iter().filter_map(|x| x.as_str()).map(s).collect(),
        None => vec![s(".")],
    };
    // A target's commands: post and test.
    let cmds = |t: &toml::Table, k: &str| -> List<Str> {
        t.get(k)
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).map(s).collect())
            .unwrap_or_default()
    };
    let rust = match t.get("rust").and_then(|r| r.as_table()) {
        Some(r) => match r.get("out").and_then(|o| o.as_str()) {
            Some(out) => MaybeRust::Some(RustTarget {
                out: s(out),
                module_dir: opt(r.get("module_dir")),
                post: cmds(r, "post"),
                test: cmds(r, "test"),
            }),
            None => return fail("[rust]: missing field `out`".into()),
        },
        None => MaybeRust::None,
    };
    let go = match t.get("go").and_then(|g| g.as_table()) {
        Some(g) => match g.get("out").and_then(|o| o.as_str()) {
            Some(out) => MaybeGo::Some(GoTarget {
                out: s(out),
                package: opt(g.get("package")),
                module: opt(g.get("module")),
                types_dir: opt(g.get("types_dir")),
                post: cmds(g, "post"),
                test: cmds(g, "test"),
            }),
            None => return fail("[go]: missing field `out`".into()),
        },
        None => MaybeGo::None,
    };
    let mut http = vec![];
    if let Some(h) = t.get("http").and_then(|h| h.as_table()) {
        for (n, v) in h {
            let Some(v) = v.as_table() else { continue };
            let Some(addr) = v.get("addr").and_then(|a| a.as_str()) else {
                return fail(format!("[http.{n}]: missing field `addr`"));
            };
            let strs = |t: &toml::Table| -> Map<Str, Str> {
                t.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (s(k), s(v))))
                    .collect()
            };
            let table = |k: &str| {
                v.get(k)
                    .and_then(|x| x.as_table())
                    .map(strs)
                    .unwrap_or_default()
            };
            http.push((
                s(n),
                ServerDecl {
                    addr: s(addr),
                    setup: opt(v.get("setup")),
                    routes: table("routes"),
                    raw: table("raw"),
                    statics: table("static"),
                    go: opt(v.get("go")),
                    rust: opt(v.get("rust")),
                },
            ));
        }
    }
    let mut bins = vec![];
    if let Some(b) = t.get("bin").and_then(|b| b.as_table()) {
        for (n, v) in b {
            let Some(v) = v.as_table() else { continue };
            bins.push((
                s(n),
                Bin {
                    entry: v.get("entry").and_then(|e| e.as_str()).map_or(s("Main"), s),
                    go: opt(v.get("go")),
                    rust: opt(v.get("rust")),
                },
            ));
        }
    }
    Ok(ResManifest::Ok(Manifest {
        name: s(name),
        sources: sources.into(),
        rust,
        has_go: t.contains_key("go"),
        go,
        bins: bins.into_iter().collect(),
        http: http.into_iter().collect(),
    }))
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
    let (t, f): (Vec<_>, Vec<_>) = (
        Path::new(&*target).components().collect(),
        Path::new(&*from).components().collect(),
    );
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
                        let full =
                            normalize(&Path::new(&*dirs[i.span.file as usize]).join(&*i.file));
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
pub fn cargo_lib_name(toml: Str, has_lib_rs: bool) -> Result<Str, String> {
    let mut section = "";
    let (mut lib, mut package) = (None, None);
    for l in toml.lines().map(str::trim) {
        if l.starts_with('[') {
            section = l;
        } else if let Some(v) = l
            .strip_prefix("name")
            .map(str::trim_start)
            .and_then(|v| v.strip_prefix('='))
        {
            let v = Some(v.trim().trim_matches('"').replace('-', "_"));
            match section {
                "[lib]" => lib = v,
                "[package]" => package = v,
                _ => {}
            }
        }
    }
    let has_lib = has_lib_rs || toml.lines().any(|l| l.trim() == "[lib]");
    Ok(if has_lib {
        lib.or(package).unwrap_or_default().into()
    } else {
        s("")
    })
}

/// The path segments from root down to dir.
pub fn rel_segs(root: Str, dir: Str) -> Result<List<Str>, String> {
    Ok(Path::new(&*dir)
        .strip_prefix(&*root)
        .map(|r| {
            r.components()
                .map(|c| Str::from(c.as_os_str().to_string_lossy().as_ref()))
                .collect()
        })
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
    Ok(paths
        .iter()
        .map(|p| {
            Path::new(&**p)
                .strip_prefix(&*root)
                .map_or(p.clone(), |r| slash(r).into())
        })
        .collect())
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

pub fn out_dir(c: Checked) -> Result<Str, String> {
    let r = unwrap_r(c.project.manifest.rust)?;
    join(c.project.root, r.out)
}

pub fn out_paths(files: List<OutFile>) -> Result<List<Str>, String> {
    Ok(files.iter().map(|f| f.path.clone()).collect())
}

pub fn dirs(root: Str, paths: List<Str>) -> Result<List<Str>, String> {
    paths
        .iter()
        .map(|p| parent(join(root.clone(), p.clone())?))
        .collect::<Result<Vec<_>, _>>()
        .map(List::from)
}

/// The program with implementation paths relative to [rust]'s out.
pub fn rebased(c: Checked) -> Result<Program, String> {
    let d = dirs(c.project.root.clone(), c.project.paths.clone())?;
    rebase_impls(c.prog.clone(), d, out_dir(c)?)
}

/// [rust]'s module_dir, or out relative to the crate's src/.
pub fn module_dir(target: RustTarget, out: Str, croot: Str) -> Result<MaybeStr, String> {
    if let MaybeStr::Some(d) = target.module_dir {
        return Ok(MaybeStr::Some(d));
    }
    if croot.is_empty() {
        return Ok(MaybeStr::None);
    }
    let segs = rel_segs(croot, out)?;
    Ok(match segs.split_first() {
        Some((src, rest)) if &**src == "src" => MaybeStr::Some(
            rest.iter()
                .map(|s| &**s)
                .collect::<Vec<_>>()
                .join("/")
                .into(),
        ),
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
    Err(format!(
        "vandorf.toml: no [bin.{only}]; bins: {}",
        if known.is_empty() {
            "none".into()
        } else {
            known.join(", ")
        }
    ))
}

pub fn bin_path(b: Bin, name: Str) -> Result<Str, String> {
    Ok(match b.rust {
        MaybeStr::Some(p) => p,
        MaybeStr::None => format!("bin/{name}.rs").into(),
    })
}

/// Fails with the errors, without the first's `error: ` (the executable
/// prints it).
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
    Ok(if text.is_empty() {
        List::default()
    } else {
        vec![OutFile {
            path: join(out, "http.rs".into())?,
            text,
            kind: "code".into(),
        }]
        .into()
    })
}

pub fn setup_name(d: ServerDecl) -> Result<Str, String> {
    Ok(match d.setup {
        MaybeStr::Some(p) => p,
        MaybeStr::None => "".into(),
    })
}

pub fn server_path(d: ServerDecl, name: Str) -> Result<Str, String> {
    Ok(match d.rust {
        MaybeStr::Some(p) => p,
        MaybeStr::None => format!("bin/{name}.rs").into(),
    })
}

/// The directory of a file under the crate's src/, out being module_dir
/// below src/.
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

/// "POST /api/calc" as a method and a path, if it is one.
pub fn split_route(route: Str, proc: Str) -> Result<List<Route>, String> {
    let Some((m, p)) = route.split_once(' ').map(|(m, p)| (m.trim(), p.trim())) else {
        return Ok(List::default());
    };
    if m.is_empty() || !m.chars().all(|c| c.is_ascii_uppercase()) || !p.starts_with('/') {
        return Ok(List::default());
    }
    Ok(vec![Route {
        method: m.into(),
        path: p.into(),
        proc,
    }]
    .into())
}

pub fn debug_quote(s: Str) -> Result<Str, String> {
    Ok(format!("{:?}", &*s).into())
}

pub fn content_type(file: Str) -> Result<Str, String> {
    let ext = Path::new(&*file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
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

// ---- Checking a manifest, as strictly as the reference's serde types ----

type Fields = &'static [(&'static str, Kind, bool)];

/// What a manifest field holds; tables are named as the reference's
/// structs, for messages.
#[derive(Clone, Copy)]
enum Kind {
    Str,
    Strs,
    /// A table of strings.
    StrMap,
    /// A table of these fields.
    Table(&'static str, Fields),
    /// A table whose values are tables of these fields.
    Tables(&'static str, Fields),
}

const BIN: Fields = &[
    ("entry", Kind::Str, false),
    ("go", Kind::Str, false),
    ("rust", Kind::Str, false),
];
const SERVER: Fields = &[
    ("addr", Kind::Str, true),
    ("setup", Kind::Str, false),
    ("routes", Kind::StrMap, false),
    ("raw", Kind::StrMap, false),
    ("static", Kind::StrMap, false),
    ("go", Kind::Str, false),
    ("rust", Kind::Str, false),
];
const META: Fields = &[("name", Kind::Str, true), ("sources", Kind::Strs, false)];
const GO: Fields = &[
    ("out", Kind::Str, true),
    ("package", Kind::Str, false),
    ("module", Kind::Str, false),
    ("types_dir", Kind::Str, false),
    ("post", Kind::Strs, false),
    ("test", Kind::Strs, false),
];
const RUST: Fields = &[
    ("out", Kind::Str, true),
    ("module_dir", Kind::Str, false),
    ("post", Kind::Strs, false),
    ("test", Kind::Strs, false),
];
const MANIFEST: Fields = &[
    ("project", Kind::Table("Meta", META), true),
    ("go", Kind::Table("GoTarget", GO), false),
    ("rust", Kind::Table("RustTarget", RUST), false),
    ("bin", Kind::Tables("Bin", BIN), false),
    ("http", Kind::Tables("Server", SERVER), false),
];

/// serde's description of a value it didn't expect.
fn unexpected(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => format!("string {s:?}"),
        toml::Value::Integer(i) => format!("integer `{i}`"),
        toml::Value::Float(f) => format!("floating point `{f}`"),
        toml::Value::Boolean(b) => format!("boolean `{b}`"),
        toml::Value::Datetime(_) | toml::Value::Table(_) => "map".into(),
        toml::Value::Array(_) => "sequence".into(),
    }
}

fn check_value(v: &toml::Value, kind: Kind) -> Result<(), String> {
    let invalid = |what: &str| Err(format!("invalid type: {}, expected {what}", unexpected(v)));
    match (kind, v) {
        (Kind::Str, toml::Value::String(_)) => Ok(()),
        (Kind::Str, _) => invalid("a string"),
        (Kind::Strs, toml::Value::Array(xs)) => {
            xs.iter().try_for_each(|x| check_value(x, Kind::Str))
        }
        (Kind::Strs, _) => invalid("a sequence"),
        (Kind::StrMap, toml::Value::Table(t)) => {
            t.values().try_for_each(|x| check_value(x, Kind::Str))
        }
        (Kind::Tables(name, fields), toml::Value::Table(t)) => t
            .values()
            .try_for_each(|x| check_value(x, Kind::Table(name, fields))),
        (Kind::StrMap | Kind::Tables(..), _) => invalid("a map"),
        (Kind::Table(_, fields), toml::Value::Table(t)) => check_table(t, fields),
        (Kind::Table(name, _), _) => invalid(&format!("struct {name}")),
    }
}

fn check_table(t: &toml::Table, fields: Fields) -> Result<(), String> {
    for (key, v) in t {
        let Some((_, kind, _)) = fields.iter().find(|f| f.0 == key) else {
            let names: Vec<String> = fields.iter().map(|f| format!("`{}`", f.0)).collect();
            let expected = match names.len() {
                1 => format!("expected {}", names[0]),
                2 => format!("expected {} or {}", names[0], names[1]),
                _ => format!("expected one of {}", names.join(", ")),
            };
            return Err(format!("unknown field `{key}`, {expected}"));
        };
        check_value(v, *kind)?;
    }
    match fields.iter().find(|f| f.2 && !t.contains_key(f.0)) {
        Some((name, ..)) => Err(format!("missing field `{name}`")),
        None => Ok(()),
    }
}

/// Why text isn't a manifest the reference would load (its fields and
/// their types), or "".
pub fn manifest_error(text: Str) -> Result<Str, String> {
    let t: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => return Ok(e.message().into()),
    };
    Ok(check_table(&t, MANIFEST).err().unwrap_or_default().into())
}

const USAGE: &str = "\
usage:
  vcc build [<bin>]            # libraries, and every bin or just <bin>
  vcc test                     # build, then each language's test commands
  vcc check
  vcc doc                      # the visualizer document, on stdout

Each works on the project: the nearest vandorf.toml in the current directory
or its parents (see the README for its fields). A project's .sys files share
one namespace. -C <dir>, anywhere in a command, runs it as if started in <dir>.

build generates what the manifest configures: [go] and [rust] libraries (the
types and the procs), an executable per [bin.<name>] running its entry proc
with the command-line arguments, and per [http.<name>] a server for each
language (which takes --addr to listen elsewhere). Impl paths in
`= \"file:function\"` are relative to the .sys file naming them; each backend
uses the reference with its extension (.go / .rs). After generating a
language, build runs that language's `post` commands; test runs each
language's `test` commands. They run in the manifest's directory; the first
that fails fails the command.

The IDE is the ide executable built alongside vcc: ide [--addr 127.0.0.1:8080],
on the working directory. It offers terminals only when listening on a
loopback address.";

/// vcc's usage, printed by --help.
pub fn usage(_n: i64) -> Result<Str, String> {
    Ok(USAGE.into())
}

/// The first argument after the command that is a flag, or "".
pub fn first_flag(args: List<Str>) -> Result<Str, String> {
    Ok(args
        .iter()
        .skip(1)
        .find(|a| a.starts_with('-'))
        .cloned()
        .unwrap_or_default())
}

/// Takes the `-C <dir>`s out of argv (anywhere, in order).
pub fn split_c(argv: List<Str>) -> Result<CliOpts, String> {
    let (mut dirs, mut rest, mut err) = (vec![], vec![], String::new());
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        if &**a != "-C" {
            rest.push(a.clone());
            continue;
        }
        match it.next() {
            Some(d) => dirs.push(d.clone()),
            None => err = "-C needs a value (see vcc --help)".into(),
        }
    }
    Ok(CliOpts {
        dirs: dirs.into(),
        rest: rest.into(),
        err: err.into(),
    })
}

// ---- Diagnostics, as the reference's vandorf::report::snippet ----

/// The nearest char boundary at or before `i`.
fn floor(text: &str, mut i: usize) -> usize {
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// One diagnostic: the message, `path:line:col`, and the line with the span
/// underlined; notes follow as `= note` lines.
pub fn snippet(
    level: Str,
    msg: Str,
    path: Str,
    text: Str,
    at: i64,
    end: i64,
    notes: List<Str>,
) -> Result<Str, String> {
    let text = &*text;
    let at = floor(text, (at.max(0) as usize).min(text.len()));
    let end = floor(text, (end.max(0) as usize).clamp(at, text.len()));
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    let line_no = text[..at].matches('\n').count() + 1;
    let col = text[line_start..at].chars().count() + 1;
    let line = text[line_start..line_end].trim_end_matches('\r');
    // Tabs shown as four spaces; the underline follows the shown text.
    let width = |s: &str| {
        s.chars()
            .map(|c| if c == '\t' { 4 } else { 1 })
            .sum::<usize>()
    };
    let shown = line.replace('\t', "    ");
    let lead = width(&line[..(at - line_start).min(line.len())]);
    let marked = width(
        &line[(at - line_start).min(line.len())..(end.min(line_end) - line_start).min(line.len())],
    )
    .max(1);
    let pad = " ".repeat(line_no.to_string().len());
    let mut out = format!(
        "{level}: {msg}\n{pad}--> {path}:{line_no}:{col}\n{pad} |\n{line_no} | {shown}\n{pad} | {}{}",
        " ".repeat(lead),
        "^".repeat(marked)
    );
    for n in notes.iter() {
        out.push_str(&format!("\n{pad} = {n}"));
    }
    Ok(out.into())
}

/// Errors as snippets (message, path:line:col, the line underlined), a
/// blank line apart (each once).
pub fn render_errors(errors: List<Diag>, paths: List<Str>, srcs: List<Str>) -> Result<Str, String> {
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    for e in errors.iter() {
        if !seen.insert((e.span.file, e.span.at, e.span.end, e.msg.clone())) {
            continue;
        }
        let f = e.span.file as usize;
        let path = paths.get(f).cloned().unwrap_or_else(|| "?".into());
        let text = srcs.get(f).cloned().unwrap_or_default();
        out.push(
            snippet(
                "error".into(),
                e.msg.clone(),
                path,
                text,
                e.span.at,
                e.span.end,
                List::default(),
            )?
            .to_string(),
        );
    }
    Ok(out.join("\n\n").into())
}

/// Fails with msg (rendered diagnostics without their first `error: `,
/// which the executable prints).
pub fn fail_report(msg: Str) -> Result<(), String> {
    Err(msg.strip_prefix("error: ").unwrap_or(&msg).to_string())
}

/// A command's result, which isn't printed: build and test say nothing when
/// they succeed.
pub fn quiet(_s: Str) -> Result<(), String> {
    Ok(())
}

/// For each func implemented in Rust: its file (absolute), function, and
/// the warning to print if the file is missing or doesn't define it.
pub fn impl_checks(
    prog: Program,
    t: Table,
    module_dir: MaybeStr,
    out: Str,
    root: Str,
    paths: List<Str>,
    srcs: List<Str>,
) -> Result<List<ImplCheck>, String> {
    let md = super::gen_leaf::trim_dir(module_dir)?;
    let types_path = match md {
        MaybeStr::Some(d) => match super::gen_leaf::module_path(d)? {
            MaybeStr::Some(m) => format!("{m}::types"),
            MaybeStr::None => "types".into(),
        },
        MaybeStr::None => "types".into(),
    };
    let out_dir = normalize(Path::new(&*out));
    let mut checks = vec![];
    for (id, n) in prog.nodes.iter().enumerate() {
        let NodeKind::Func(_) = &n.kind else { continue };
        let MaybeImpl::Some(i) = super::gen_leaf::rs_impl(n.clone(), "rs".into())? else {
            continue;
        };
        let path = normalize(&out_dir.join(&*i.file));
        let shown = relative(slash(&path).into(), root.clone())?;
        let qual = super::sema_leaf::qual_name(n.clone())?;
        let sig = super::gen_leaf::rust_signature(
            prog.clone(),
            t.clone(),
            id as i64,
            i.function.clone(),
            types_path.as_str().into(),
        )?;
        let f = n.span.file as usize;
        let warn = |msg: String| -> Result<Str, String> {
            snippet(
                "warning".into(),
                msg.into(),
                paths.get(f).cloned().unwrap_or_else(|| "?".into()),
                srcs.get(f).cloned().unwrap_or_default(),
                n.span.at,
                n.span.end,
                vec![Str::from(format!("expected: {sig}").as_str())].into(),
            )
        };
        checks.push(ImplCheck {
            path: slash(&path).into(),
            function: i.function.clone(),
            keyword: "fn".into(),
            missing: warn(format!(
                "{qual} is implemented in {shown}, which does not exist"
            ))?,
            undefined: warn(format!(
                "{qual} is implemented by fn {} in {shown}, which does not define it",
                i.function
            ))?,
        });
    }
    Ok(checks.into())
}

/// Whether src defines `fn function` (or a generic `fn function<` /
/// `func Function[T any](`); keyword is fn or func.
pub fn defines_fn(src: Str, function: Str, keyword: Str) -> Result<bool, String> {
    Ok(["(", "<", "["]
        .iter()
        .any(|open| src.contains(&format!("{keyword} {function}{open}"))))
}

pub fn has_rust(m: Manifest) -> Result<bool, String> {
    Ok(matches!(m.rust, MaybeRust::Some(_)))
}

pub fn go_tests(m: Manifest) -> Result<List<Str>, String> {
    Ok(match m.go {
        MaybeGo::Some(g) => g.test,
        MaybeGo::None => List::default(),
    })
}

pub fn rust_tests(m: Manifest) -> Result<List<Str>, String> {
    Ok(match m.rust {
        MaybeRust::Some(r) => r.test,
        MaybeRust::None => List::default(),
    })
}

pub fn unwrap_or(m: MaybeStr, d: Str) -> Result<Str, String> {
    Ok(match m {
        MaybeStr::Some(s) => s,
        MaybeStr::None => d,
    })
}

pub fn go_out_dir(root: Str, g: GoTarget) -> Result<Str, String> {
    join(root, g.out)
}

/// The program with implementation paths relative to out.
pub fn rebased_to(c: Checked, out: Str) -> Result<Program, String> {
    let d = dirs(c.project.root.clone(), c.project.paths.clone())?;
    rebase_impls(c.prog.clone(), d, out)
}

/// [go]'s package, or the output directory's name.
pub fn go_package(g: GoTarget, out: Str) -> Result<Str, String> {
    if let MaybeStr::Some(p) = g.package {
        return Ok(p);
    }
    let base = Path::new(&*out)
        .file_name()
        .map_or("generated".into(), |n| n.to_string_lossy().into_owned());
    let s: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    Ok(if s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("p{s}")
    } else {
        s
    }
    .into())
}

/// [go]'s module, else go.mod's module path (root: go.mod's directory, ""
/// if none) followed by out's directories below root.
pub fn go_import_base(g: GoTarget, root: Str, gomod: Str, out: Str) -> Result<MaybeStr, String> {
    if let MaybeStr::Some(m) = g.module {
        return Ok(MaybeStr::Some(m));
    }
    if root.is_empty() {
        return Ok(MaybeStr::None);
    }
    let Some(module) = gomod.lines().find_map(|l| l.trim().strip_prefix("module ")) else {
        return Ok(MaybeStr::None);
    };
    let mut segs = vec![module.trim().trim_matches('"').to_string()];
    segs.extend(rel_segs(root, out)?.iter().map(|s| s.to_string()));
    Ok(MaybeStr::Some(segs.join("/").into()))
}

/// Each bin's main package: [bin.*]'s go, by default <go.out>/../cmd/<name>,
/// relative to go.out.
pub fn go_main_specs(
    c: Checked,
    g: GoTarget,
    names: List<Str>,
) -> Result<List<GoMainSpec>, String> {
    let root = c.project.root.clone();
    let out = join(root.clone(), g.out.clone())?;
    names
        .iter()
        .map(|name| {
            let b = &c.project.manifest.bins[name];
            let dir = match &b.go {
                MaybeStr::Some(d) => d.to_string(),
                MaybeStr::None => format!("{}/../cmd/{name}", g.out),
            };
            Ok(GoMainSpec {
                name: name.clone(),
                entry: b.entry.clone(),
                path: relative(join(root.clone(), dir.into())?, out.clone())?,
            })
        })
        .collect::<Result<Vec<_>, String>>()
        .map(List::from)
}

/// A server's main package: [http.*]'s go, by default
/// <go.out>/../cmd/<name>, relative to go.out.
pub fn go_server_path(c: Checked, g: GoTarget, name: Str) -> Result<Str, String> {
    let root = c.project.root.clone();
    let out = join(root.clone(), g.out.clone())?;
    let dir = match &c.project.manifest.http[&name].go {
        MaybeStr::Some(d) => d.to_string(),
        MaybeStr::None => format!("{}/../cmd/{name}", g.out),
    };
    relative(join(root, dir.into())?, out)
}

pub fn no_go_specs(_n: i64) -> Result<ResGoServerSpecList, String> {
    Ok(ResGoServerSpecList::Ok(List::default()))
}

pub fn go_out_files(out: Str, files: List<GoFile>) -> Result<List<OutFile>, String> {
    files
        .iter()
        .map(|f| {
            Ok(OutFile {
                path: join(join(out.clone(), f.dir.clone())?, f.name.clone())?,
                text: f.text.clone(),
                kind: f.kind.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()
        .map(List::from)
}

/// For each func implemented in Go: as impl_checks, for `func`.
#[allow(clippy::too_many_arguments)]
pub fn go_impl_checks(
    prog: Program,
    t: Table,
    g: GoTarget,
    out: Str,
    import_base: MaybeStr,
    root: Str,
    paths: List<Str>,
    srcs: List<Str>,
) -> Result<List<ImplCheck>, String> {
    let package = go_package(g.clone(), out.clone())?;
    let types_dir = unwrap_or(g.types_dir.clone(), "types".into())?;
    let setup = super::go_leaf::go_setup(package, import_base, types_dir)?;
    let out_dir = normalize(Path::new(&*out));
    let mut checks = vec![];
    for (id, n) in prog.nodes.iter().enumerate() {
        let NodeKind::Func(_) = &n.kind else { continue };
        let MaybeImpl::Some(i) = super::gen_leaf::rs_impl(n.clone(), "go".into())? else {
            continue;
        };
        let path = normalize(&out_dir.join(&*i.file));
        let shown = relative(slash(&path).into(), root.clone())?;
        let qual = super::sema_leaf::qual_name(n.clone())?;
        let sig = super::go_leaf::go_signature(
            prog.clone(),
            t.clone(),
            setup.tq.clone(),
            id as i64,
            i.function.clone(),
        )?;
        let f = n.span.file as usize;
        let warn = |msg: String| -> Result<Str, String> {
            snippet(
                "warning".into(),
                msg.into(),
                paths.get(f).cloned().unwrap_or_else(|| "?".into()),
                srcs.get(f).cloned().unwrap_or_default(),
                n.span.at,
                n.span.end,
                vec![Str::from(format!("expected: {sig}").as_str())].into(),
            )
        };
        checks.push(ImplCheck {
            path: slash(&path).into(),
            function: i.function.clone(),
            keyword: "func".into(),
            missing: warn(format!(
                "{qual} is implemented in {shown}, which does not exist"
            ))?,
            undefined: warn(format!(
                "{qual} is implemented by func {} in {shown}, which does not define it",
                i.function
            ))?,
        });
    }
    Ok(checks.into())
}
