//! vcc's IDE (../sys/ide), called directly: routes, auth, compile.

use vcc::vcc::types as v;

fn req(method: &str, path: &str, query: &[(&str, &str)], headers: &[(&str, &str)], body: &str) -> v::HttpResponse {
    let m = |kv: &[(&str, &str)]| kv.iter().map(|(k, x)| (v::Str::from(*k), v::Str::from(*x))).collect();
    let r = v::HttpRequest { method: method.into(), path: path.into(), query: m(query), headers: m(headers), body: body.into() };
    vcc::vcc::run_ide(r).unwrap()
}

#[test]
fn serves_pages_with_the_token_and_guards_writes() {
    // The source directory is the working directory: the crate's.
    let page = req("GET", "/viewer", &[], &[], "");
    assert_eq!(page.status, 200);
    assert!(!page.body.contains("__VANDORF_TOKEN__") && page.headers.get(&v::Str::from("Cache-Control")).map(|s| &**s) == Some("no-store"));
    let save = req("POST", "/api/save", &[], &[("host", "127.0.0.1:8080")], r#"{"path": "x.sys", "source": ""}"#);
    assert_eq!((save.status, &*save.body), (403, r#"{"error":"forbidden: missing or wrong session token","ok":false}"#));
    // Called in-process, not listening anywhere: not on a loopback address,
    // so no terminals.
    let term = req("GET", "/api/term/list", &[], &[("host", "127.0.0.1")], "");
    assert_eq!((term.status, &*term.body), (400, r#"{"error":"terminals are off: the IDE isn't listening on a loopback address","ok":false}"#));
    assert_eq!(req("GET", "/nope", &[], &[], "").status, 404);
}

#[test]
fn reads_sources_and_compiles() {
    let src = req("GET", "/api/source", &[("path", "src/lib.rs")], &[], "");
    assert_eq!(src.status, 200);
    assert!(src.body.starts_with(r#"{"ok":true,"path":"src/lib.rs","source":"#), "{}", src.body);
    let esc = req("GET", "/api/source", &[("path", "../../../../../etc/passwd")], &[], "");
    assert_eq!(esc.status, 400);
    let c = req("POST", "/api/compile", &[], &[], r#"{"source": "proc Main(A): int { func A(n: int): int; A -> id(this + 1); }"}"#);
    assert_eq!(c.status, 200);
    assert!(c.body.contains(r#""grids":1"#) && c.body.contains(r#""ok":true"#) && c.body.contains(r#""signature""#) == false, "{}", &c.body[..200]);
    let e = req("POST", "/api/compile", &[], &[], r#"{"source": "proc P(A): int { func A(): string; }"}"#);
    assert!(e.body.contains(r#""diagnostics":[{"file":"<editor>","from":{"ch":5,"line":0},"here":true,"msg":"P.A ends proc P but returns string, expected int""#), "{}", e.body);
}
