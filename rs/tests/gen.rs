//! vcc's code generators (../sys/gen): the type table and the generated Rust
//! and Go (code, mains and HTTP servers) for each valid corpus program and the
//! demo project, against their golden files (first recorded from the
//! reference's src/codegen). Each program's golden file holds every case
//! tried on it, its files or its error.

mod common;

use vcc::vcc::types as v;

/// Programs to generate from: (name, program), the corpus's that check and
/// the demo project.
fn programs() -> Vec<(String, v::Program)> {
    let mut out: Vec<(String, v::Program)> = common::corpus()
        .into_iter()
        .filter_map(|(name, src)| common::analyze(&[&src]).ok().map(|p| (name, p)))
        .collect();
    let srcs = common::project_sources("examples/demo");
    let srcs: Vec<&str> = srcs.iter().map(|s| s.as_str()).collect();
    out.push(("examples-demo".to_string(), common::analyze(&srcs).unwrap()));
    out
}

/// Top-level procs' names, in source order.
fn top_procs(p: &v::Program) -> Vec<String> {
    p.top.iter().map(|&n| p.nodes[n as usize].qual.last().unwrap().to_string()).collect()
}

/// Files as golden text: a header per file, then its text.
fn files_text<'a>(files: impl IntoIterator<Item = (String, &'a str)>) -> String {
    files.into_iter().map(|(name, text)| format!("==== {name}\n{text}\n")).collect()
}

fn v_tyref(r: &v::TyRef) -> String {
    match r {
        v::TyRef::Prim(p) => p.to_string(),
        v::TyRef::Array(e) => format!("[]{}", v_tyref(e)),
        v::TyRef::Map(m) => format!("map[{}]{}", v_tyref(&m.key), v_tyref(&m.value)),
        v::TyRef::Named(i) => format!("#{i}"),
        v::TyRef::Foreign(i) => format!("foreign#{i}"),
    }
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

fn rust_generate(prog: &v::Program, servers: Vec<v::ServerSpec>) -> v::RustOut {
    vcc::vcc::run_rust_generate(v::RustGenerateS { prog: prog.clone(), module_dir: v::MaybeStr::Some("app".into()), servers: servers.into() }).unwrap()
}

#[test]
fn builds_type_tables() {
    let progs = programs();
    assert!(progs.len() > 15);
    for (name, prog) in &progs {
        let got = v_table(&vcc::vcc::run_build_table(prog.clone()).unwrap());
        common::check("gen-table", name, &(got.join("\n") + "\n"));
    }
}

#[test]
fn generates_rust_types() {
    for (name, prog) in &programs() {
        // Only programs whose Rust generates.
        if let v::MaybeStr::Some(_) = rust_generate(prog, vec![]).err {
            assert!(!common::has("gen-rust-types", name), "{name} has golden types but no longer generates");
            continue;
        }
        let table = vcc::vcc::run_build_table(prog.clone()).unwrap();
        common::check("gen-rust-types", name, &vcc::vcc::run_rust_typegen(table).unwrap().to_string());
    }
}

#[test]
fn generates_rust_procs() {
    let mut n = 0;
    for (name, prog) in &programs() {
        let got = rust_generate(prog, vec![]);
        match &got.err {
            v::MaybeStr::None => {
                common::check("gen-rust", name, &files_text([("procs.rs".to_string(), &*got.procs), ("mod.rs".to_string(), &*got.modrs), ("types.rs".to_string(), &*got.types)]));
                n += 1;
            }
            v::MaybeStr::Some(g) => common::check("gen-rust", name, &format!("error: {g}\n")),
        }
    }
    assert!(n > 15, "{n}");
}

type Route = (String, String, String);

/// Server cases for a program: every top-level proc routed, a missing one, a
/// raw route, and with a setup (the corpus's setup program) each mistake.
fn server_cases(procs: &[String]) -> Vec<(Option<String>, Vec<Route>, Vec<Route>)> {
    let routes: Vec<Route> = procs.iter().enumerate().map(|(i, p)| (if i % 2 == 0 { "POST" } else { "GET" }.to_string(), format!("/api/{}", p.to_lowercase()), p.clone())).collect();
    let raw: Vec<Route> = procs.iter().take(1).map(|p| ("GET".to_string(), "/raw".to_string(), p.clone())).collect();
    let route = |m: &str, p: &str, proc: &str| (m.to_string(), p.to_string(), proc.to_string());
    let mut cases = vec![(None, routes, vec![]), (None, vec![route("GET", "/x", "Missing")], vec![]), (None, vec![], raw)];
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
    cases
}

fn v_routes(rs: &[Route]) -> v::List<v::Route> {
    rs.iter().map(|(m, p, pr)| v::Route { method: m.as_str().into(), path: p.as_str().into(), proc: pr.as_str().into() }).collect()
}

#[test]
fn generates_http() {
    let (mut n, mut with_setup) = (0, 0);
    for (name, prog) in &programs() {
        let mut golden = String::new();
        for (setup, routes, raw) in server_cases(&top_procs(prog)) {
            golden += &format!("######## setup {setup:?}, routes {routes:?}, raw {raw:?}\n");
            let vsrv = v::ServerSpec {
                name: "web-api".into(),
                addr: "127.0.0.1:9".into(),
                setup: setup.as_deref().unwrap_or("").into(),
                cli: "".into(),
                routes: v_routes(&routes),
                raw: v_routes(&raw),
                statics: vec![v::StaticFile { url: "/".into(), file: "../../web/index.html".into(), ctype: "text/html; charset=utf-8".into() }].into(),
                path: "bin/web.rs".into(),
            };
            let got = rust_generate(prog, vec![vsrv.clone()]);
            match &got.err {
                v::MaybeStr::None => {
                    let bin = vcc::vcc::run_rust_server(v::RustServerS { prog: prog.clone(), srv: vsrv, module_dir: v::MaybeStr::Some("app".into()), krate: "demo".into() }).unwrap();
                    golden += &files_text([("http.rs".to_string(), &*got.http), ("mod.rs".to_string(), &*got.modrs), (format!("{}/{}", bin.dir, bin.file), &*bin.text)]);
                    n += 1;
                    with_setup += setup.is_some() as usize;
                }
                v::MaybeStr::Some(g) => golden += &format!("error: {g}\n"),
            }
        }
        common::check("gen-http", name, &golden);
    }
    assert!(n > 15, "{n}");
    assert!(with_setup > 0, "no server with a setup generated");
}

/// The Go backend's files, as golden text (by directory and name), or its
/// error.
fn go_text(out: v::GoOut) -> Result<String, String> {
    if let v::MaybeStr::Some(e) = out.err {
        return Err(e.to_string());
    }
    let mut files: Vec<((String, String), String)> = out.files.iter().map(|f| ((f.dir.to_string(), f.name.to_string()), f.text.to_string())).collect();
    files.sort();
    Ok(files_text(files.iter().map(|((d, f), t)| (format!("{d}/{f}"), t.as_str()))))
}

#[test]
fn generates_go() {
    let (mut generated, mut errors) = (0, 0);
    for (name, prog) in &programs() {
        let base = Some("example.com/app".to_string());
        // Types in their own package; in the code package without an import
        // path; a separate types package without one (an error); then each
        // top-level proc (the first few) as a main, and a missing one.
        let mut cases: Vec<(String, Option<String>, String, Vec<(String, String, String)>)> = vec![
            ("app".into(), base.clone(), "types".into(), vec![]),
            ("app".into(), None, ".".into(), vec![]),
            ("app".into(), None, "types/".into(), vec![]),
        ];
        for p in top_procs(prog).into_iter().take(8) {
            cases.push(("main".into(), base.clone(), "types".into(), vec![("bin".into(), p, "../cmd/bin".into())]));
        }
        cases.push(("app".into(), base.clone(), "types".into(), vec![("bin".into(), "Missing".into(), "cmd".into())]));
        let mut golden = String::new();
        for (package, import_base, types_dir, ms) in cases {
            golden += &format!("######## package {package}, import {import_base:?}, types {types_dir}, mains {:?}\n", ms.iter().map(|m| &m.1).collect::<Vec<_>>());
            let got = go_text(
                vcc::vcc::run_go_generate(v::GoGenIn {
                    prog: prog.clone(),
                    package: package.as_str().into(),
                    import_base: import_base.as_deref().map_or(v::MaybeStr::None, |b| v::MaybeStr::Some(b.into())),
                    types_dir: types_dir.as_str().into(),
                    mains: ms.iter().map(|(n, e, p)| v::GoMainSpec { name: n.as_str().into(), entry: e.as_str().into(), path: p.as_str().into() }).collect(),
                    servers: v::List::default(),
                    imports: v::Map::default(),
                })
                .unwrap(),
            );
            match got {
                Ok(t) => {
                    golden += &t;
                    generated += 1;
                }
                Err(e) => {
                    golden += &format!("error: {e}\n");
                    errors += 1;
                }
            }
        }
        common::check("gen-go", name, &golden);
    }
    assert!(generated > 100 && errors > 15, "{generated} generated, {errors} errors");
}

#[test]
fn generates_go_servers() {
    let (mut n, mut with_setup) = (0, 0);
    for (name, prog) in &programs() {
        let mut golden = String::new();
        for (setup, routes, raw) in server_cases(&top_procs(prog)) {
            golden += &format!("######## setup {setup:?}, routes {routes:?}, raw {raw:?}\n");
            // Static contents that need escaping, and a second server without any.
            let text = "<p class=\"x\">a\tb\\c\r\n`d` \u{1} é</p>\n";
            let server = |name: &str, path: &str, full: bool| v::GoServerSpec {
                name: name.into(),
                addr: "127.0.0.1:9".into(),
                setup: if full { setup.as_deref().unwrap_or("") } else { "" }.into(),
                cli: "".into(),
                routes: v_routes(&routes),
                raw: if full { v_routes(&raw) } else { v::List::default() },
                statics: if full {
                    vec![v::GoStatic { url: "/".into(), file: "web/index.html".into(), ctype: "text/html; charset=utf-8".into(), text: text.into() }].into()
                } else {
                    v::List::default()
                },
                path: path.into(),
            };
            let got = go_text(
                vcc::vcc::run_go_generate(v::GoGenIn {
                    prog: prog.clone(),
                    package: "app".into(),
                    import_base: v::MaybeStr::Some("example.com/app".into()),
                    types_dir: "types".into(),
                    mains: v::List::default(),
                    servers: vec![server("web-api", "../cmd/web-api", true), server("plain", "../cmd/plain", false)].into(),
                    imports: v::Map::default(),
                })
                .unwrap(),
            );
            match got {
                Ok(t) => {
                    assert!(t.contains("==== /http_vandorf.go\n"), "{name}");
                    golden += &t;
                    n += 1;
                    with_setup += setup.is_some() as usize;
                }
                Err(e) => golden += &format!("error: {e}\n"),
            }
        }
        common::check("gen-go-servers", name, &golden);
    }
    assert!(n > 15, "{n}");
    assert!(with_setup > 0, "no server with a setup generated");
}

/// A server's cli: run on a command instead of serving, in Rust and Go; a
/// missing proc; one with the wrong signature.
#[test]
fn generates_cli_mains() {
    let src = "func Say(s: string): void;\nproc Cli(argv: []string): void {\n\tCli -> Say(s = \"hi\");\n}\nproc Takes(n: int): void {\n\tTakes -> Say(s = \"hi\");\n}\nproc Gives(argv: []string): int {\n\tGives -> id(1);\n}\nproc Hi(): string {\n\tHi -> id(\"hi\");\n}\n";
    let prog = common::analyze(&[src]).unwrap();
    let mut golden = String::new();
    for cli in ["Cli", "Missing", "Takes", "Gives"] {
        golden += &format!("######## cli {cli}\n");
        let routes = v_routes(&[("GET".to_string(), "/hi".to_string(), "Hi".to_string())]);
        let vsrv = v::ServerSpec {
            name: "web".into(),
            addr: "127.0.0.1:9".into(),
            setup: "".into(),
            cli: cli.into(),
            routes: routes.clone(),
            raw: v::List::default(),
            statics: v::List::default(),
            path: "bin/web.rs".into(),
        };
        let bin = vcc::vcc::run_rust_server(v::RustServerS { prog: prog.clone(), srv: vsrv, module_dir: v::MaybeStr::Some("app".into()), krate: "demo".into() }).unwrap();
        golden += &match &bin.err {
            v::MaybeStr::None => files_text([(format!("{}/{}", bin.dir, bin.file), &*bin.text)]),
            v::MaybeStr::Some(e) => format!("rust error: {e}\n"),
        };
        let go = vcc::vcc::run_go_generate(v::GoGenIn {
            prog: prog.clone(),
            package: "app".into(),
            import_base: v::MaybeStr::Some("example.com/app".into()),
            types_dir: "types".into(),
            mains: v::List::default(),
            servers: vec![v::GoServerSpec { name: "web".into(), addr: "127.0.0.1:9".into(), setup: "".into(), cli: cli.into(), routes, raw: v::List::default(), statics: v::List::default(), path: "cmd/web".into() }].into(),
            imports: v::Map::default(),
        })
        .unwrap();
        golden += &match go_text(go) {
            Ok(text) => text.split("==== ").filter(|f| f.starts_with("cmd/web/")).map(|f| format!("==== {f}")).collect::<String>(),
            Err(e) => format!("go error: {e}\n"),
        };
    }
    common::check("gen-cli", "cli", &golden);
}
