//! vcc's IDE (../sys/ide), through its server's routes: each to its own
//! proc. Routes, auth, compile.

use vcc::vcc::http;

struct Res {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Res {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// The response for a request, as the server routes it; 404 for a route it
/// doesn't have, as the server answers.
fn req(method: &str, path: &str, query: &[(&str, &str)], headers: &[(&str, &str)], body: &str) -> Res {
    let r = http::Request {
        method: method.into(),
        path: path.into(),
        query: query.iter().map(|(k, x)| format!("{k}={x}")).collect::<Vec<_>>().join("&"),
        headers: headers.iter().map(|(k, x)| (k.to_lowercase(), x.to_string())).collect(),
        body: body.into(),
    };
    match http::handle_ide(&r) {
        Some(r) => Res { status: r.status, headers: r.headers, body: String::from_utf8(r.body).unwrap() },
        None => Res { status: 404, headers: vec![], body: String::new() },
    }
}

#[test]
fn serves_pages_with_the_token_and_guards_writes() {
    // The source directory is the working directory: the crate's.
    let page = req("GET", "/viewer", &[], &[], "");
    assert_eq!(page.status, 200);
    assert!(!page.body.contains("__VANDORF_TOKEN__") && page.header("Cache-Control") == Some("no-store"));
    let save = req("POST", "/api/save", &[], &[("host", "127.0.0.1:8080")], r#"{"path": "x.sys", "source": ""}"#);
    assert_eq!((save.status, save.body.as_str()), (403, r#"{"error":"forbidden: missing or wrong session token","ok":false}"#));
    // Called in-process, not listening anywhere: not on a loopback address,
    // so no terminals.
    let term = req("GET", "/api/term/list", &[], &[("host", "127.0.0.1")], "");
    assert_eq!((term.status, term.body.as_str()), (400, r#"{"error":"terminals are off: the IDE isn't listening on a loopback address","ok":false}"#));
    assert_eq!(req("GET", "/nope", &[], &[], "").status, 404);
}

#[test]
fn reads_sources_and_compiles() {
    let src = req("GET", "/api/source", &[("path", "src/lib.rs")], &[], "");
    assert_eq!(src.status, 200);
    assert!(src.body.starts_with(r#"{"ok":true,"path":"src/lib.rs","source":"#), "{}", src.body);
    let esc = req("GET", "/api/source", &[("path", "../../../../../etc/passwd")], &[], "");
    assert_eq!(esc.status, 400);
    let c = req("POST", "/api/compile", &[], &[], r#"{"source": "proc Main(n: int): int { func A(n: int): int; Main -> A(n = this.n); A -> id(this + 1); }"}"#);
    assert_eq!(c.status, 200);
    assert!(c.body.contains(r#""grids":1"#) && c.body.contains(r#""ok":true"#) && c.body.contains(r#""signature""#) == false, "{}", &c.body[..200]);
    let e = req("POST", "/api/compile", &[], &[], r#"{"source": "proc P(): int { func A(): string; P -> A(); }"}"#);
    assert!(e.body.contains(r#""diagnostics":[{"file":"<editor>","from":{"ch":5,"line":0},"here":true,"msg":"P.A ends proc P but returns string, expected int""#), "{}", e.body);
}

#[test]
fn overview_shows_each_servers_command_line() {
    // A project under the working directory, so the IDE serves it.
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ide-overview");
    std::fs::create_dir_all(dir.join("sys")).unwrap();
    std::fs::write(dir.join("sys/a.sys"), "// Runs a command.\nproc Cli(argv: []string): void { func Out(argv: []string): void; Cli -> Out(argv = this.argv); }\nproc Page(n: int): int { func A(n: int): int; Page -> A(n = this.n); }\n").unwrap();
    std::fs::write(
        dir.join("vandorf.toml"),
        "[project]\nname = \"o\"\nsources = [\"sys\"]\n[go]\nout = \"go/app\"\n[rust]\nout = \"rs/src/app\"\n\
         [http.web]\naddr = \"127.0.0.1:1\"\ncli = \"Cli\"\nroutes = { \"GET /p\" = \"Page\" }\n\
         [http.api]\naddr = \"127.0.0.1:2\"\nrust = \"bin/apid.rs\"\n",
    )
    .unwrap();
    let cwd = std::env::current_dir().unwrap();
    let rel = dir.strip_prefix(dunce::canonicalize(&cwd).unwrap()).or_else(|_| dir.strip_prefix(&cwd)).unwrap();
    let res = req("GET", "/api/project", &[("dir", rel.to_str().unwrap())], &[], "");
    assert_eq!(res.status, 200, "{}", res.body);
    let b = &res.body;
    // A server with a cli: its executable, and the proc a command runs.
    assert!(b.contains(r#""exe":"web""#) && b.contains(r#""go":"go/cmd/web""#) && b.contains(r#""rust":"bin/web.rs""#), "{b}");
    assert!(b.contains(r#""cli":"Cli","cli_at":{"doc":"Runs a command.","line":1,"path":"target/tmp/ide-overview/sys/a.sys"}"#), "{b}");
    // Without: its executable named after its Rust bin.
    assert!(b.contains(r#""exe":"apid""#) && b.contains(r#""cli":null,"cli_at":null"#), "{b}");
}
