//! Leaves for the IDE (the .sys files here): responses, paths, positions, and the
//! JSON the pages read.

use crate::vcc::types::*;
use std::path::{Component, Path, PathBuf};

fn s(x: &str) -> Str {
    x.into()
}

fn obj(fs: Vec<(&str, Json)>) -> Json {
    Json::Obj(
        fs.into_iter()
            .map(|(k, v)| JsonField {
                key: k.into(),
                value: v,
            })
            .collect(),
    )
}

fn jstr(x: &str) -> Json {
    Json::Str(x.into())
}

fn headers(hs: &[(&str, &str)]) -> Map<Str, Str> {
    hs.iter().map(|(k, v)| (s(k), s(v))).collect()
}

fn response(status: i64, hs: &[(&str, &str)], body: Str) -> HttpResponse {
    HttpResponse {
        status,
        headers: headers(hs),
        body,
    }
}

// ---- Responses ----

/// Maps a path to the IDE page served there ("" if none), and to the one
/// carrying the token.
pub fn page_name(path: Str) -> Result<Str, String> {
    Ok(s(match &*path {
        "/" => "ide",
        "/explorer" => "explorer",
        "/viewer" => "viewer",
        "/terminal" => "terminal",
        "/workbench" => "workbench",
        "/viz" | "/viz/" => "viz",
        "/project" => "project",
        _ => "",
    }))
}

/// Serves a page as HTML, with the session token put in for the requests that
/// need it.
pub fn page_response(text: Str, token: Str) -> Result<HttpResponse, String> {
    Ok(response(
        200,
        &[
            ("Content-Type", "text/html; charset=utf-8"),
            ("Cache-Control", "no-store"),
        ],
        text.replace("__VANDORF_TOKEN__", &token).into(),
    ))
}

/// Answers with JSON: 200 and the JSON (ApiErr: 400 and {"ok": false,
/// "error": msg}; Forbidden: 403).
pub fn api(j: Json) -> Result<HttpResponse, String> {
    Ok(response(
        200,
        &[("Content-Type", "application/json")],
        super::doc_leaf::compact(j)?,
    ))
}

fn api_status(status: i64, msg: &str) -> Result<HttpResponse, String> {
    let body =
        super::doc_leaf::compact(obj(vec![("ok", Json::Bool(false)), ("error", jstr(msg))]))?;
    Ok(response(
        status,
        &[("Content-Type", "application/json")],
        body,
    ))
}

pub fn api_err(msg: Str) -> Result<HttpResponse, String> {
    api_status(400, &msg)
}

pub fn forbidden(_n: i64) -> Result<HttpResponse, String> {
    api_status(403, "forbidden: missing or wrong session token")
}

pub fn ok_json(_n: i64) -> Result<HttpResponse, String> {
    api(obj(vec![("ok", Json::Bool(true))]))
}

pub fn error_json(msg: Str) -> Result<Json, String> {
    Ok(obj(vec![
        ("ok", Json::Bool(false)),
        ("error", Json::Str(msg)),
    ]))
}

pub(super) fn is_loopback(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h == "localhost" || h == "::1" || h.starts_with("127.")
}

/// Checks a write's headers: the session's token, and on a loopback address a
/// loopback Host, against DNS rebinding.
pub fn authorized(headers: Map<Str, Str>, token: Str, loopback: bool) -> Result<bool, String> {
    let host_ok = !loopback
        || headers.get(&s("host")).is_some_and(|h| {
            let name = match h.rsplit_once(':') {
                Some((n, p)) if p.chars().all(|c| c.is_ascii_digit()) => n,
                _ => h,
            };
            is_loopback(name)
        });
    Ok(host_ok && headers.get(&s("x-vandorf-token")) == Some(&token))
}

pub fn chunk_response(c: TermChunk) -> Result<HttpResponse, String> {
    let next = c.next.to_string();
    Ok(response(
        200,
        &[
            ("Content-Type", "application/octet-stream"),
            ("X-Next", &next),
            ("X-Closed", if c.closed { "1" } else { "0" }),
        ],
        c.data,
    ))
}

// ---- Values ----

pub fn strs(xs: List<Str>) -> Result<List<Json>, String> {
    Ok(xs.iter().map(|x| Json::Str(x.clone())).collect())
}

pub fn ints(xs: List<i64>) -> Result<List<Json>, String> {
    Ok(xs.iter().map(|x| Json::Num(*x)).collect())
}

pub fn unwrap_j(r: ResJson) -> Result<Json, String> {
    match r {
        ResJson::Ok(j) => Ok(j),
        ResJson::Err(e) => Err(e.msg.to_string()),
    }
}

pub fn unwrap_res_string(r: ResString) -> Result<Str, String> {
    match r {
        ResString::Ok(x) => Ok(x),
        ResString::Err(e) => Err(e.msg.to_string()),
    }
}

pub fn unwrap_out(r: ResOutFileList) -> Result<List<OutFile>, String> {
    match r {
        ResOutFileList::Ok(x) => Ok(x),
        ResOutFileList::Err(e) => Err(e.msg.to_string()),
    }
}

pub fn unwrap_project(p: MaybeProject) -> Result<Project, String> {
    match p {
        MaybeProject::Some(p) => Ok(p),
        MaybeProject::None => Err("no project".into()),
    }
}

pub fn or_empty(r: ResString) -> Result<Str, String> {
    Ok(match r {
        ResString::Ok(x) => x,
        ResString::Err(_) => s(""),
    })
}

pub fn non_blank(m: MaybeStr) -> Result<bool, String> {
    Ok(matches!(m, MaybeStr::Some(x) if !x.trim().is_empty()))
}

pub fn non_empty(m: MaybeStr) -> Result<bool, String> {
    Ok(matches!(m, MaybeStr::Some(x) if !x.is_empty()))
}

/// Reads q[k] as an int, or -1 if it isn't one.
pub fn query_int(s: QueryIntS) -> Result<i64, String> {
    Ok(s.q.get(&s.k).and_then(|v| v.parse().ok()).unwrap_or(-1))
}

/// Whether the query names a terminal: an id that's a number, not negative.
pub fn has_id(q: Map<Str, Str>) -> Result<bool, String> {
    Ok(q.get(&s("id"))
        .and_then(|v| v.parse::<i64>().ok())
        .is_some_and(|i| i >= 0))
}

pub fn byte_len(s: Str) -> Result<i64, String> {
    Ok(s.len() as i64)
}

pub fn bad_path(p: Str) -> Result<bool, String> {
    Ok(p.is_empty() || p.contains('\0'))
}

pub fn is_sys(p: Str) -> Result<bool, String> {
    Ok(Path::new(&*p)
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("sys")))
}

pub fn is_manifest(p: Str) -> Result<bool, String> {
    Ok(Path::new(&*p)
        .file_name()
        .is_some_and(|n| n == "vandorf.toml"))
}

pub fn set_at(xs: List<Str>, i: i64, x: Str) -> Result<List<Str>, String> {
    let mut v = xs.to_vec();
    v[i as usize] = x;
    Ok(v.into())
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

pub fn index_of_path(paths: List<Str>, root: Str, abs: Str) -> Result<i64, String> {
    let abs = normalize(Path::new(&*abs));
    Ok(paths
        .iter()
        .position(|p| normalize(&Path::new(&*root).join(&**p)) == abs)
        .map_or(-1, |i| i as i64))
}

/// Names the editor's file in messages: its path, its "name", or <editor>.
pub fn editor_name(j: Json) -> Result<Str, String> {
    let get = |k: &str| match super::doc_leaf::json_str(j.clone(), k.into()) {
        Ok(MaybeStr::Some(x)) if !(k == "path" && x.is_empty()) => Some(x),
        _ => None,
    };
    Ok(get("path")
        .or_else(|| get("name"))
        .unwrap_or_else(|| s("<editor>")))
}

/// Whether q[k] is a terminal dimension: an integer in 1..=65535.
pub fn is_dim(s: QueryIntS) -> Result<bool, String> {
    Ok(s.q
        .get(&s.k)
        .and_then(|v| v.parse::<u16>().ok())
        .is_some_and(|n| n > 0))
}

/// Makes a cache key of what a unit is checked from, its sources: two hashes
/// of them, so a collision is out of reach. Their names, and the file asked
/// about, only change how the result is shown.
pub fn check_key(srcs: List<Str>) -> Result<Str, String> {
    use std::hash::{Hash, Hasher};
    let hash = |seed: &str| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        seed.hash(&mut h);
        for s in srcs.iter() {
            (**s).hash(&mut h);
        }
        h.finish()
    };
    Ok(format!("{:016x}{:016x}", hash("a"), hash("b")).into())
}

/// Gives the program of a check that failed to parse: an empty one.
pub fn no_program(_n: i64) -> Result<Program, String> {
    Ok(Program {
        types: List::default(),
        nodes: List::default(),
        globals: Map::default(),
        top: List::default(),
    })
}

/// Whether a compile request wants generated code: unless it says `previews:
/// false`.
pub fn wants_previews(j: Json) -> Result<bool, String> {
    Ok(
        !matches!(&j, Json::Obj(fs) if fs.iter().any(|f| &*f.key == "previews" && f.value == Json::Bool(false))),
    )
}

pub fn project_name_json(p: MaybeProject) -> Result<Json, String> {
    Ok(match p {
        MaybeProject::Some(p) => Json::Str(p.manifest.name),
        MaybeProject::None => Json::Null,
    })
}

pub fn unwrap_go(g: MaybeGo) -> Result<GoTarget, String> {
    match g {
        MaybeGo::Some(g) => Ok(g),
        MaybeGo::None => Err("no [go]".into()),
    }
}

pub fn nominal_go_mains(prog: Program) -> Result<List<GoMainSpec>, String> {
    Ok(if super::gen_leaf::find_top(prog, s("Main"))? >= 0 {
        vec![GoMainSpec {
            name: s("main"),
            entry: s("Main"),
            path: s("main"),
        }]
        .into()
    } else {
        List::default()
    })
}

pub fn go_output_json(o: GoOut, prog: Program, t: Table, setup: GoSetup) -> Result<Json, String> {
    if let MaybeStr::Some(e) = o.err {
        return error_json(e);
    }
    let files = |fs: Vec<&GoFile>| {
        obj(fs
            .into_iter()
            .map(|f| (&*f.name, Json::Str(f.text.clone())))
            .collect())
    };
    let of = |kind: &str| {
        o.files
            .iter()
            .filter(|f| &*f.kind == kind)
            .collect::<Vec<_>>()
    };
    let artifact = |kind: &str, dir: Str, fs: Vec<&GoFile>| {
        obj(vec![
            ("kind", jstr(kind)),
            ("dir", Json::Str(dir)),
            ("files", files(fs)),
        ])
    };
    let mut artifacts = vec![
        artifact("types", setup.dir.clone(), of("types")),
        artifact("code", s(""), of("code")),
    ];
    for m in of("main") {
        artifacts.push(artifact("main", m.dir.clone(), vec![m]));
    }
    let mut impls = vec![];
    for (id, n) in prog.nodes.iter().enumerate() {
        let NodeKind::Func(_) = &n.kind else { continue };
        let MaybeImpl::Some(i) = super::gen_leaf::rs_impl(n.clone(), s("go"))? else {
            continue;
        };
        impls.push(obj(vec![
            ("node", Json::Str(super::sema_leaf::qual_name(n.clone())?)),
            ("file", Json::Str(i.file.clone())),
            ("function", Json::Str(i.function.clone())),
            (
                "signature",
                Json::Str(super::go_leaf::go_signature(
                    prog.clone(),
                    t.clone(),
                    setup.tq.clone(),
                    id as i64,
                    i.function.clone(),
                )?),
            ),
        ]));
    }
    Ok(obj(vec![
        ("ok", Json::Bool(true)),
        ("artifacts", Json::Arr(artifacts.into())),
        ("impls", Json::Arr(impls.into())),
    ]))
}

// ---- Positions ----

/// 0-based line and UTF-16 column of a byte offset, as CodeMirror expects.
fn line_col_of(src: &str, offset: i64) -> (i64, i64) {
    let mut offset = (offset.max(0) as usize).min(src.len());
    while !src.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &src[..offset];
    let line = before.matches('\n').count() as i64;
    let col = before
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .encode_utf16()
        .count() as i64;
    (line, col)
}

pub fn line_col(src: Str, offset: i64) -> Result<LineColOut, String> {
    let (line, ch) = line_col_of(&src, offset);
    Ok(LineColOut { line, ch })
}

fn pos(src: &str, offset: i64) -> Json {
    let (line, ch) = line_col_of(src, offset);
    obj(vec![("line", Json::Num(line)), ("ch", Json::Num(ch))])
}

pub fn diagnostics_json(
    names: List<Str>,
    srcs: List<Str>,
    here: i64,
    errors: List<Diag>,
) -> Result<Json, String> {
    Ok(Json::Arr(
        errors
            .iter()
            .map(|d| {
                let f = d.span.file as usize;
                let src = srcs.get(f).map_or("", |x| &**x);
                obj(vec![
                    ("msg", Json::Str(d.msg.clone())),
                    ("file", Json::Str(names.get(f).cloned().unwrap_or_default())),
                    ("here", Json::Bool(d.span.file == here)),
                    ("from", pos(src, d.span.at)),
                    ("to", pos(src, d.span.end)),
                ])
            })
            .collect(),
    ))
}

pub fn render_diagnostics(
    names: List<Str>,
    srcs: List<Str>,
    errors: List<Diag>,
) -> Result<Str, String> {
    super::cli_leaf::render_errors(errors, names, srcs)
}

// ---- The outline ----

pub fn qualify(prefix: Str, name: Str) -> Result<Str, String> {
    Ok(if prefix.is_empty() {
        name
    } else {
        format!("{prefix}.{name}").into()
    })
}

fn item(
    kind: &str,
    name: &Ident,
    prefix: &str,
    src: &str,
    doc: &Str,
    children: List<Item>,
) -> Item {
    Item {
        kind: kind.into(),
        name: name.name.clone(),
        qual: qualify(prefix.into(), name.name.clone()).unwrap(),
        line: line_col_of(src, name.span.at).0,
        doc: doc.clone(),
        children,
    }
}

pub fn type_items(src: Str, ts: List<TypeDecl>) -> Result<List<Item>, String> {
    Ok(ts
        .iter()
        .map(|t| item("type", &t.name, "", &src, &t.doc, List::default()))
        .collect())
}

pub fn iface_items(src: Str, is: List<Interface>) -> Result<List<Item>, String> {
    Ok(is
        .iter()
        .map(|i| {
            item(
                "interface",
                &i.name,
                "",
                &src,
                &i.doc,
                i.funcs
                    .iter()
                    .map(|f| item("func", &f.name, &i.name.name, &src, &f.doc, List::default()))
                    .collect(),
            )
        })
        .collect())
}

/// Takes a func's or proc's doc comment.
pub fn node_doc(n: Node) -> Result<Str, String> {
    Ok(match n {
        Node::Func(f) => f.doc,
        Node::Proc(p) => p.doc,
    })
}

pub fn by_line(items: List<Item>) -> Result<List<Item>, String> {
    let mut v = items.to_vec();
    v.sort_by_key(|i| i.line);
    Ok(v.into())
}

fn item_json(i: &Item) -> Json {
    obj(vec![
        ("kind", Json::Str(i.kind.clone())),
        ("name", Json::Str(i.name.clone())),
        ("qual", Json::Str(i.qual.clone())),
        ("line", Json::Num(i.line)),
        ("doc", Json::Str(i.doc.clone())),
        (
            "children",
            Json::Arr(i.children.iter().map(item_json).collect()),
        ),
    ])
}

pub fn items_json(items: List<Item>) -> Result<Json, String> {
    Ok(Json::Arr(items.iter().map(item_json).collect()))
}

// ---- Symbols ----

fn file_of(unit: &Unit, span: &Span) -> (PathBuf, usize) {
    let f = span.file as usize;
    (
        normalize(&Path::new(&*unit.root).join(unit.names.get(f).map_or("", |x| &**x))),
        f,
    )
}

fn rel(base: &str, p: &Path) -> Json {
    match p.strip_prefix(base) {
        Ok(r) => Json::Str(slash(r).into()),
        Err(_) => Json::Null,
    }
}

/// Places a node for the viewer: { file, line }, its file relative to base
/// (or null).
pub fn loc_json(base: Str, unit: Unit, span: Span) -> Result<Json, String> {
    let (abs, f) = file_of(&unit, &span);
    let line = line_col_of(unit.srcs.get(f).map_or("", |x| &**x), span.at).0;
    Ok(obj(vec![
        ("file", rel(&base, &abs)),
        ("line", Json::Num(line)),
    ]))
}

/// Lists a func's implementations for the viewer: each as written, its path
/// (relative to base), function and language.
pub fn imps_json(base: Str, unit: Unit, span: Span, imps: List<ImplRef>) -> Result<Json, String> {
    let (abs, _) = file_of(&unit, &span);
    let dir = abs
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(&*base));
    Ok(Json::Arr(
        imps.iter()
            .map(|i| {
                let p = normalize(&dir.join(&*i.file));
                obj(vec![
                    ("written", Json::Str(i.file.clone())),
                    ("path", rel(&base, &p)),
                    ("function", Json::Str(i.function.clone())),
                    ("lang", jstr(i.file.rsplit_once('.').map_or("", |(_, e)| e))),
                ])
            })
            .collect(),
    ))
}

pub fn func_imps(n: NodeDecl) -> Result<List<ImplRef>, String> {
    Ok(match n.kind {
        NodeKind::Func(f) => f.imps,
        _ => List::default(),
    })
}

pub fn entry_name(prog: Program, pid: i64) -> Result<Str, String> {
    match &prog.nodes[pid as usize].kind {
        NodeKind::Proc(p) => Ok(prog.nodes[p.entry as usize]
            .qual
            .last()
            .cloned()
            .unwrap_or_default()),
        _ => Err("entry_name: not a proc".into()),
    }
}

// ---- The preview ----

pub fn nominal_mains(prog: Program) -> Result<Map<Str, Bin>, String> {
    Ok(if super::gen_leaf::find_top(prog, s("Main"))? >= 0 {
        Map::from_iter([(
            s("main"),
            Bin {
                entry: s("Main"),
                go: MaybeStr::None,
                rust: MaybeStr::Some(s("main.rs")),
            },
        )])
    } else {
        Map::default()
    })
}

pub fn output_json(
    types: Str,
    modrs: Str,
    procs: Str,
    mains: List<OutFile>,
    impls: Json,
) -> Result<Json, String> {
    let files =
        |fs: Vec<(&str, Str)>| obj(fs.into_iter().map(|(k, v)| (k, Json::Str(v))).collect());
    let mut artifacts = vec![
        obj(vec![
            ("kind", jstr("types")),
            ("dir", jstr("")),
            ("files", files(vec![("types.rs", types)])),
        ]),
        obj(vec![
            ("kind", jstr("code")),
            ("dir", jstr("")),
            ("files", files(vec![("mod.rs", modrs), ("procs.rs", procs)])),
        ]),
    ];
    for m in mains.iter() {
        // PreviewMains keeps the dir in path and the file name in kind.
        artifacts.push(obj(vec![
            ("kind", jstr("main")),
            ("dir", Json::Str(m.path.clone())),
            ("files", obj(vec![(&m.kind, Json::Str(m.text.clone()))])),
        ]));
    }
    Ok(obj(vec![
        ("ok", Json::Bool(true)),
        ("artifacts", Json::Arr(artifacts.into())),
        ("impls", impls),
    ]))
}

pub fn impls_json(prog: Program, t: Table, module_dir: MaybeStr) -> Result<Json, String> {
    let md = super::gen_leaf::trim_dir(module_dir)?;
    let types_path = match md {
        MaybeStr::Some(d) => match super::gen_leaf::module_path(d)? {
            MaybeStr::Some(m) => format!("{m}::types"),
            MaybeStr::None => "types".into(),
        },
        MaybeStr::None => "types".into(),
    };
    let mut out = vec![];
    for (id, n) in prog.nodes.iter().enumerate() {
        let NodeKind::Func(_) = &n.kind else { continue };
        let MaybeImpl::Some(i) = super::gen_leaf::rs_impl(n.clone(), s("rs"))? else {
            continue;
        };
        let qual = super::sema_leaf::qual_name(n.clone())?;
        let sig = super::gen_leaf::rust_signature(
            prog.clone(),
            t.clone(),
            id as i64,
            i.function.clone(),
            types_path.as_str().into(),
        )?;
        out.push(obj(vec![
            ("node", Json::Str(qual)),
            ("file", Json::Str(i.file.clone())),
            ("function", Json::Str(i.function.clone())),
            ("signature", Json::Str(sig)),
        ]));
    }
    Ok(Json::Arr(out.into()))
}

// ---- Projects ----

pub fn css_response(text: Str) -> Result<HttpResponse, String> {
    Ok(response(
        200,
        &[
            ("Content-Type", "text/css; charset=utf-8"),
            ("Cache-Control", "no-store"),
        ],
        text,
    ))
}

fn some_str(m: &MaybeStr) -> Option<&Str> {
    match m {
        MaybeStr::Some(x) => Some(x),
        MaybeStr::None => None,
    }
}

/// What runs each proc: bins, then each server's routes and raw routes.
fn entry_points(m: &Manifest) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut out: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (name, b) in m.bins.iter() {
        out.entry(b.entry.to_string())
            .or_default()
            .push(format!("bin {name}"));
    }
    for (_, srv) in m.http.iter() {
        for (route, proc) in srv.routes.iter() {
            out.entry(proc.to_string())
                .or_default()
                .push(route.to_string());
        }
        for (route, proc) in srv.raw.iter() {
            out.entry(proc.to_string())
                .or_default()
                .push(format!("{route} (raw)"));
        }
    }
    out
}

fn entries_json(m: &Manifest) -> Json {
    Json::Obj(
        entry_points(m)
            .into_iter()
            .map(|(k, v)| JsonField {
                key: k.into(),
                value: Json::Arr(v.iter().map(|x| jstr(x)).collect()),
            })
            .collect(),
    )
}

/// Summarizes a manifest for the explorer: { name, dir, entries }; "?" and no
/// entries if the manifest doesn't load.
pub fn project_info(text: ResString, dir: Str) -> Result<Json, String> {
    let m = match text {
        ResString::Ok(t) => match super::cli_leaf::parse_manifest(t)? {
            ResManifest::Ok(m) => Some(m),
            ResManifest::Err(_) => None,
        },
        ResString::Err(_) => None,
    };
    let (name, entries) = match &m {
        Some(m) => (Json::Str(m.name.clone()), entries_json(m)),
        None => (jstr("?"), Json::Obj(List::default())),
    };
    Ok(obj(vec![
        ("name", name),
        ("dir", Json::Str(dir)),
        ("entries", entries),
    ]))
}

fn field_of<'a>(j: &'a Json, key: &str) -> Option<&'a Json> {
    match j {
        Json::Obj(fs) => fs.iter().find(|f| &*f.key == key).map(|f| &f.value),
        _ => None,
    }
}

/// Marks the top-level proc items something runs with an "entry": what runs
/// them.
pub fn with_entries(items: Json, project: Json) -> Result<Json, String> {
    let Some(Json::Obj(entries)) = field_of(&project, "entries") else {
        return Ok(items);
    };
    let Json::Arr(xs) = items else {
        return Ok(items);
    };
    Ok(Json::Arr(
        xs.iter()
            .map(|it| {
                let is_proc = matches!(field_of(it, "kind"), Some(Json::Str(k)) if &**k == "proc");
                let by = match field_of(it, "name") {
                    Some(Json::Str(n)) => entries
                        .iter()
                        .find(|f| f.key == *n)
                        .map(|f| f.value.clone()),
                    _ => None,
                };
                match (it, is_proc, by) {
                    (Json::Obj(fs), true, Some(by)) => Json::Obj(
                        fs.clone().concat(
                            &vec![JsonField {
                                key: "entry".into(),
                                value: by,
                            }]
                            .into(),
                        ),
                    ),
                    _ => it.clone(),
                }
            })
            .collect(),
    ))
}

fn count(items: &[Item], n: &mut (i64, i64, i64)) {
    for it in items {
        match &*it.kind {
            "proc" => n.0 += 1,
            "func" => n.1 += 1,
            "type" => n.2 += 1,
            _ => {}
        }
        count(&it.children, n);
    }
}

fn opt_json(m: &MaybeStr) -> Json {
    some_str(m).map_or(Json::Null, |x| Json::Str(x.clone()))
}

/// Builds a project's overview from its manifest and its files' outlines:
/// name, settings, bins and servers with the procs they run, files and
/// counts.
pub fn project_json(
    base: Str,
    p: Project,
    text: Str,
    outlines: List<Outlined>,
) -> Result<Json, String> {
    let m = &p.manifest;
    let rel = |path: &Path| -> String {
        path.strip_prefix(&*base)
            .map(slash)
            .unwrap_or_else(|_| slash(path))
    };
    let root = PathBuf::from(&*p.root);
    let files: Vec<String> = p
        .paths
        .iter()
        .map(|f| rel(&normalize(&root.join(&**f))))
        .collect();
    let mut procs: std::collections::HashMap<String, Json> = Default::default();
    let mut n = (0, 0, 0);
    let mut unparsed = vec![];
    for (o, path) in outlines.iter().zip(&files) {
        if !o.ok {
            unparsed.push(jstr(path));
            continue;
        }
        count(&o.items, &mut n);
        for it in o.items.iter().filter(|it| &*it.kind == "proc") {
            procs.insert(
                it.name.to_string(),
                obj(vec![
                    ("path", jstr(path)),
                    ("line", Json::Num(it.line)),
                    ("doc", Json::Str(it.doc.clone())),
                ]),
            );
        }
    }
    let at = |proc: &str| procs.get(proc).cloned().unwrap_or(Json::Null);
    let bins: Vec<Json> = m
        .bins
        .iter()
        .map(|(name, b)| {
            let go = match &m.go {
                MaybeGo::Some(g) => {
                    let d = some_str(&b.go)
                        .map(|x| x.to_string())
                        .unwrap_or_else(|| format!("{}/../cmd/{name}", g.out));
                    let full = normalize(&root.join(d));
                    Json::Str(
                        super::cli_leaf::relative(slash(&full).into(), p.root.clone()).unwrap(),
                    )
                }
                MaybeGo::None => Json::Null,
            };
            let rust = match &m.rust {
                MaybeRust::Some(_) => Json::Str(
                    some_str(&b.rust)
                        .cloned()
                        .unwrap_or_else(|| format!("bin/{name}.rs").into()),
                ),
                MaybeRust::None => Json::Null,
            };
            obj(vec![
                ("name", Json::Str(name.clone())),
                ("entry", Json::Str(b.entry.clone())),
                ("at", at(&b.entry)),
                ("go", go),
                ("rust", rust),
            ])
        })
        .collect();
    let route = |r: &str, proc: &str, raw: bool| {
        let (method, path) = r.split_once(' ').unwrap_or(("", r));
        obj(vec![
            ("method", jstr(method)),
            ("path", jstr(path)),
            ("proc", jstr(proc)),
            ("raw", Json::Bool(raw)),
            ("at", at(proc)),
        ])
    };
    let servers: Vec<Json> = m
        .http
        .iter()
        .map(|(name, srv)| {
            let mut routes: Vec<Json> =
                srv.routes.iter().map(|(r, p)| route(r, p, false)).collect();
            routes.extend(srv.raw.iter().map(|(r, p)| route(r, p, true)));
            obj(vec![
                ("name", Json::Str(name.clone())),
                ("addr", Json::Str(srv.addr.clone())),
                (
                    "rust",
                    Json::Str(
                        some_str(&srv.rust)
                            .cloned()
                            .unwrap_or_else(|| format!("bin/{name}.rs").into()),
                    ),
                ),
                ("routes", Json::Arr(routes.into())),
                (
                    "statics",
                    Json::Arr(
                        srv.statics
                            .iter()
                            .map(|(u, f)| {
                                obj(vec![
                                    ("path", Json::Str(u.clone())),
                                    ("file", Json::Str(f.clone())),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ])
        })
        .collect();
    let go = match &m.go {
        MaybeGo::Some(g) => obj(vec![
            ("out", Json::Str(g.out.clone())),
            ("package", opt_json(&g.package)),
            ("module", opt_json(&g.module)),
            ("types_dir", opt_json(&g.types_dir)),
        ]),
        MaybeGo::None => Json::Null,
    };
    let rust = match &m.rust {
        MaybeRust::Some(r) => obj(vec![
            ("out", Json::Str(r.out.clone())),
            ("module_dir", opt_json(&r.module_dir)),
        ]),
        MaybeRust::None => Json::Null,
    };
    Ok(obj(vec![
        ("ok", Json::Bool(true)),
        ("name", Json::Str(m.name.clone())),
        ("dir", jstr(&rel(&root))),
        ("manifest", jstr(&rel(&root.join("vandorf.toml")))),
        ("text", Json::Str(text)),
        (
            "sources",
            Json::Arr(m.sources.iter().map(|x| Json::Str(x.clone())).collect()),
        ),
        ("go", go),
        ("rust", rust),
        ("bins", Json::Arr(bins.into())),
        ("servers", Json::Arr(servers.into())),
        ("files", Json::Arr(files.iter().map(|f| jstr(f)).collect())),
        ("unparsed", Json::Arr(unparsed.into())),
        (
            "counts",
            obj(vec![
                ("files", Json::Num(p.paths.len() as i64)),
                ("procs", Json::Num(n.0)),
                ("funcs", Json::Num(n.1)),
                ("types", Json::Num(n.2)),
            ]),
        ),
    ]))
}
