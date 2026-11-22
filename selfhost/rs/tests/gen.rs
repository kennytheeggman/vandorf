//! vcc's code generators (../sys/gen) against the reference
//! (src/codegen): the same type table and the same generated files.

use vandorf::codegen::types as rt;
use vcc::vcc::types as v;

pub fn analyze(srcs: &[&str]) -> v::Program {
    let mut all = v::File { types: Default::default(), interfaces: Default::default(), nodes: Default::default() };
    for (i, s) in srcs.iter().enumerate() {
        let f = vcc::vcc::run_parse_file((*s).into(), i as i64).unwrap();
        all = v::File { types: all.types.concat(&f.types), interfaces: all.interfaces.concat(&f.interfaces), nodes: all.nodes.concat(&f.nodes) };
    }
    let a = vcc::vcc::run_analyze(v::AnalyzeAtS { srcs: srcs.iter().map(|s| v::Str::from(*s)).collect(), file: all }).unwrap();
    assert!(a.errors.is_empty(), "{:?}", a.errors);
    a.prog
}

/// Programs to generate from: (name, reference program, sources).
pub fn programs() -> Vec<(String, vandorf::sema::Program, Vec<String>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = vec![];
    let text = std::fs::read_to_string(root.join("tests/pipeline.rs")).unwrap();
    for part in text.split("r#\"").skip(1) {
        let src = part.split("\"#").next().unwrap();
        if let Ok(prog) = vandorf::compile("t.sys", src) {
            out.push((src.to_string(), prog, vec![src.to_string()]));
        }
    }
    let src = std::fs::read_to_string(root.join("tests/fixtures/test1.sys")).unwrap();
    out.push(("test1.sys".into(), vandorf::compile("t.sys", &src).unwrap(), vec![src]));
    for dir in ["selfhost", "examples/demo"] {
        let p = vandorf::project::load(&root.join(dir).join("vandorf.toml")).unwrap();
        let prog = vandorf::check_files(&p.files).unwrap();
        out.push((dir.to_string(), prog, p.files.iter().map(|f| f.text.clone()).collect()));
    }
    out
}

fn r_tyref(r: &rt::TyRef) -> String {
    match r {
        rt::TyRef::Prim(p) => format!("{p:?}").to_lowercase(),
        rt::TyRef::Array(e) => format!("[]{}", r_tyref(e)),
        rt::TyRef::Map(k, val) => format!("map[{}]{}", r_tyref(k), r_tyref(val)),
        rt::TyRef::Named(i) => format!("#{i}"),
    }
}

fn v_tyref(r: &v::TyRef) -> String {
    match r {
        v::TyRef::Prim(p) => p.to_string(),
        v::TyRef::Array(e) => format!("[]{}", v_tyref(e)),
        v::TyRef::Map(m) => format!("map[{}]{}", v_tyref(&m.key), v_tyref(&m.value)),
        v::TyRef::Named(i) => format!("#{i}"),
    }
}

fn r_table(t: &rt::TypeTable) -> Vec<String> {
    let mut out = vec![];
    for (i, n) in t.types.iter().enumerate() {
        let (kind, ms) = match &n.shape {
            rt::Shape::Struct(ms) => ("struct", ms),
            rt::Shape::Enum(ms) => ("enum", ms),
        };
        let ms: Vec<String> = ms.iter().map(|(f, r)| format!("{f}{}: {}", if t.is_boxed(i, f) { "*" } else { "" }, r_tyref(r))).collect();
        out.push(format!("#{i} {} {kind} {{ {} }} ({})", n.name, ms.join(", "), n.origin));
    }
    for (n, r) in &t.aliases {
        out.push(format!("alias {n} = {}", r_tyref(r)));
    }
    out
}

fn v_table(t: &v::Table) -> Vec<String> {
    let mut out = vec![];
    for (i, n) in t.types.iter().enumerate() {
        let (kind, ms) = match &n.shape {
            v::Shape::Struct(ms) => ("struct", ms),
            v::Shape::Enum(ms) => ("enum", ms),
        };
        let ms: Vec<String> = ms.iter().map(|m| format!("{}{}: {}", m.name, if t.boxed.contains_key(&v::Str::from(format!("{i}:{}", m.name))) { "*" } else { "" }, v_tyref(&m.r))).collect();
        out.push(format!("#{i} {} {kind} {{ {} }} ({})", n.name, ms.join(", "), n.origin));
    }
    for a in t.aliases.iter() {
        out.push(format!("alias {} = {}", a.name, v_tyref(&a.r)));
    }
    out
}

#[test]
fn builds_type_tables_like_the_reference() {
    let progs = programs();
    assert!(progs.len() > 15);
    for (name, prog, srcs) in &progs {
        let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
        let want = r_table(&rt::TypeTable::build(prog));
        let got = v_table(&vcc::vcc::run_build_table(analyze(&srcs)).unwrap());
        if want != got {
            let i = want.iter().zip(&got).position(|(a, b)| a != b);
            panic!("{name}\nfirst difference at {i:?}:\n  reference: {:?}\n  vcc:       {:?}\n({} vs {} lines)", i.map(|i| &want[i]), i.map(|i| &got[i]), want.len(), got.len());
        }
    }
}

#[test]
fn generates_rust_types_like_the_reference() {
    use vandorf::codegen::rust;
    for (name, prog, srcs) in &programs() {
        let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
        let Ok(out) = rust::generate(prog, &rust::RustOptions { module_dir: Some("app".into()), ..Default::default() }) else { continue };
        let want = &out.types.files["types.rs"];
        let table = vcc::vcc::run_build_table(analyze(&srcs)).unwrap();
        let got = vcc::vcc::run_rust_typegen(table).unwrap().to_string();
        if *want != got {
            let line = want.lines().zip(got.lines()).position(|(a, b)| a != b);
            panic!("{name}: types.rs differs at line {line:?}: {:?} vs {:?}", line.map(|l| want.lines().nth(l)), line.map(|l| got.lines().nth(l)));
        }
    }
}

#[test]
fn generates_rust_procs_like_the_reference() {
    use vandorf::codegen::rust;
    let mut n = 0;
    for (name, prog, srcs) in &programs() {
        let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
        let want = rust::generate(prog, &rust::RustOptions { module_dir: Some("app".into()), ..Default::default() });
        let got = vcc::vcc::run_rust_generate(v::RustGenerateAtS { prog: analyze(&srcs), module_dir: v::MaybeStr::Some("app".into()), servers: Default::default() }).unwrap();
        match (want, &got.err) {
            (Ok(out), v::MaybeStr::None) => {
                for (file, text) in [("procs.rs", &got.procs), ("mod.rs", &got.modrs), ("types.rs", &got.types)] {
                    let want = out.code.files.get(file).or(out.types.files.get(file)).unwrap();
                    if **want != **text {
                        let line = want.lines().zip(text.lines()).position(|(a, b)| a != b);
                        panic!("{name}: {file} differs at line {line:?}:\n  reference: {:?}\n  vcc:       {:?}", line.and_then(|l| want.lines().nth(l)), line.and_then(|l| text.lines().nth(l)));
                    }
                }
                n += 1;
            }
            (Err(e), v::MaybeStr::Some(g)) => assert_eq!(e, g.to_string(), "{name}"),
            (w, g) => panic!("{name}: reference {:?}, vcc {g:?}", w.map(|_| ())),
        }
    }
    assert!(n > 15, "{n}");
}

#[test]
fn generates_http_like_the_reference() {
    use vandorf::codegen::rust;
    let (mut n, mut with_setup) = (0, 0);
    for (name, prog, srcs) in &programs() {
        let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
        // A server routing to every top-level proc, and one to a missing one.
        let procs: Vec<String> = prog.top_procs.iter().map(|&p| prog.node(p).name().to_string()).collect();
        let routes: Vec<(String, String, String)> = procs.iter().enumerate().map(|(i, p)| (if i % 2 == 0 { "POST" } else { "GET" }.to_string(), format!("/api/{}", p.to_lowercase()), p.clone())).collect();
        let raw: Vec<(String, String, String)> = procs.iter().take(1).map(|p| ("GET".to_string(), "/raw".to_string(), p.clone())).collect();
        let route = |m: &str, p: &str, proc: &str| (m.to_string(), p.to_string(), proc.to_string());
        let mut cases: Vec<(Option<String>, Vec<(String, String, String)>, Vec<(String, String, String)>)> =
            vec![(None, routes.clone(), vec![]), (None, vec![route("GET", "/x", "Missing")], vec![]), (None, vec![], raw)];
        // With a setup (the pipeline's setup program): ctx passed, and each mistake.
        if procs.iter().any(|p| p == "Setup") {
            let setup = |p: &str| Some(p.to_string());
            cases.extend([
                (setup("Setup"), vec![route("GET", "/hi", "Hi")], vec![route("GET", "/", "Page")]),
                (setup("Nope"), vec![route("GET", "/hi", "Hi")], vec![]),
                (setup("Takes"), vec![route("GET", "/hi", "Hi")], vec![]),
                (setup("Setup"), vec![route("GET", "/x", "NoCtx")], vec![]),
                (setup("Setup"), vec![route("GET", "/x", "WrongCtx")], vec![]),
                (setup("Setup"), vec![route("GET", "/x", "Missing")], vec![]),
                (setup("Setup"), vec![], vec![route("GET", "/", "NoCtx")]),
                (setup("Setup"), vec![], vec![route("GET", "/", "Hi")]),
            ]);
        }
        for (setup, routes, raw) in cases {
            let srv = rust::ServerSpec {
                name: "web-api".into(),
                addr: "127.0.0.1:9".into(),
                setup: setup.clone(),
                routes: routes.clone(),
                raw: raw.clone(),
                statics: vec![("/".into(), "../../web/index.html".into(), "text/html; charset=utf-8".into())],
                path: "bin/web.rs".into(),
            };
            let opts = rust::RustOptions { module_dir: Some("app".into()), crate_name: Some("demo".into()), servers: vec![srv.clone()], ..Default::default() };
            let want = rust::generate(prog, &opts);
            let vsrv = v::ServerSpec {
                name: srv.name.as_str().into(),
                addr: srv.addr.as_str().into(),
                setup: setup.as_deref().unwrap_or("").into(),
                routes: routes.iter().map(|(m, p, pr)| v::Route { method: m.as_str().into(), path: p.as_str().into(), proc: pr.as_str().into() }).collect(),
                raw: raw.iter().map(|(m, p, pr)| v::Route { method: m.as_str().into(), path: p.as_str().into(), proc: pr.as_str().into() }).collect(),
                statics: srv.statics.iter().map(|(u, f, c)| v::StaticFile { url: u.as_str().into(), file: f.as_str().into(), ctype: c.as_str().into() }).collect(),
                path: srv.path.as_str().into(),
            };
            let got = vcc::vcc::run_rust_generate(v::RustGenerateAtS { prog: analyze(&srcs), module_dir: v::MaybeStr::Some("app".into()), servers: vec![vsrv.clone()].into() }).unwrap();
            match (want, &got.err) {
                (Ok(out), v::MaybeStr::None) => {
                    for (file, text) in [("http.rs", &got.http), ("mod.rs", &got.modrs)] {
                        let want = &out.code.files[file];
                        if **want != **text {
                            let line = want.lines().zip(text.lines()).position(|(a, b)| a != b);
                            panic!("{name}: {file} differs at line {line:?}:\n  reference: {:?}\n  vcc:       {:?}", line.and_then(|l| want.lines().nth(l)), line.and_then(|l| text.lines().nth(l)));
                        }
                    }
                    let bin = vcc::vcc::run_rust_server(v::RustServerAtS { srv: vsrv, module_dir: v::MaybeStr::Some("app".into()), krate: "demo".into() }).unwrap();
                    let main = out.mains.last().unwrap();
                    assert_eq!((main.dir.as_str(), main.files.keys().next().unwrap().as_str()), (&*bin.dir, &*bin.file), "{name}");
                    assert_eq!(main.files["web.rs"], *bin.text, "{name}");
                    n += 1;
                    with_setup += setup.is_some() as usize;
                }
                (Err(e), v::MaybeStr::Some(g)) => assert_eq!(e, g.to_string(), "{name}"),
                (w, g) => panic!("{name}: reference {:?}, vcc {g:?}", w.map(|_| ())),
            }
        }
    }
    assert!(n > 15, "{n}");
    assert!(with_setup > 0, "no server with a setup generated");
}

/// The Go backend's files as (directory, name) -> text, or its error.
fn go_files(out: Result<vandorf::codegen::Output, String>) -> Result<std::collections::BTreeMap<(String, String), String>, String> {
    let out = out?;
    let mut files = std::collections::BTreeMap::new();
    for art in std::iter::once(&out.types).chain([&out.code]).chain(&out.mains) {
        for (name, text) in &art.files {
            files.insert((art.dir.clone(), name.clone()), text.clone());
        }
    }
    Ok(files)
}

fn vcc_go_files(out: v::GoOut) -> Result<std::collections::BTreeMap<(String, String), String>, String> {
    if let v::MaybeStr::Some(e) = out.err {
        return Err(e.to_string());
    }
    Ok(out.files.iter().map(|f| ((f.dir.to_string(), f.name.to_string()), f.text.to_string())).collect())
}

#[test]
fn generates_go_like_the_reference() {
    use vandorf::codegen::{MainSpec, go::GoOptions};
    let (mut files, mut errors, mut mains) = (0, 0, 0);
    for (name, prog, srcs) in &programs() {
        let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
        let vprog = analyze(&srcs);
        let base = Some("example.com/app".to_string());
        // Types in their own package; in the code package without an import
        // path; a separate types package without one (an error); then each
        // top-level procs (the first few) as mains.
        let mut cases: Vec<(String, Option<String>, String, Vec<MainSpec>)> = vec![
            ("app".into(), base.clone(), "types".into(), vec![]),
            ("app".into(), None, ".".into(), vec![]),
            ("app".into(), None, "types/".into(), vec![]),
        ];
        for &p in prog.top_procs.iter().take(8) {
            let entry = prog.node(p).name().to_string();
            let main = MainSpec { name: "bin".into(), entry, path: "../cmd/bin".into() };
            cases.push(("main".into(), base.clone(), "types".into(), vec![main]));
        }
        cases.push(("app".into(), base.clone(), "types".into(), vec![MainSpec { name: "bin".into(), entry: "Missing".into(), path: "cmd".into() }]));
        for (package, import_base, types_dir, ms) in cases {
            let mut opts = GoOptions::new(package.as_str(), import_base.clone());
            opts.types_dir = types_dir.clone();
            opts.mains = ms.clone();
            let want = go_files(vandorf::codegen::go::generate(prog, &opts));
            let got = vcc_go_files(
                vcc::vcc::run_go_generate(v::GoGenIn {
                    prog: vprog.clone(),
                    package: package.as_str().into(),
                    import_base: import_base.as_deref().map_or(v::MaybeStr::None, |b| v::MaybeStr::Some(b.into())),
                    types_dir: types_dir.as_str().into(),
                    mains: ms.iter().map(|m| v::GoMainSpec { name: m.name.as_str().into(), entry: m.entry.as_str().into(), path: m.path.as_str().into() }).collect(),
                })
                .unwrap(),
            );
            let case = format!("{name} (package {package}, import {import_base:?}, types {types_dir}, mains {:?})", ms.iter().map(|m| &m.entry).collect::<Vec<_>>());
            match (want, got) {
                (Ok(want), Ok(got)) => {
                    assert_eq!(want.keys().collect::<Vec<_>>(), got.keys().collect::<Vec<_>>(), "{case}");
                    for (file, text) in &want {
                        if *text != got[file] {
                            let line = text.lines().zip(got[file].lines()).position(|(a, b)| a != b);
                            panic!(
                                "{case}: {file:?} differs at line {line:?}:\n  reference: {:?}\n  vcc:       {:?}",
                                line.and_then(|l| text.lines().nth(l)),
                                line.and_then(|l| got[file].lines().nth(l))
                            );
                        }
                    }
                    files += want.len();
                    mains += ms.len();
                }
                (Err(w), Err(g)) => {
                    assert_eq!(w, g, "{case}");
                    errors += 1;
                }
                (w, g) => panic!("{case}: reference {:?}, vcc {:?}", w.map(|_| ()), g.map(|_| ())),
            }
        }
    }
    assert!(files > 100 && errors > 15 && mains > 10, "{files} files, {errors} errors, {mains} mains");
}
