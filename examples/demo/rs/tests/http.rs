//! The demo's HTTP routes ([http.api]), called without a network.

use vandorf_demo::app::http::{Request, handle_api};

fn call(method: &str, path: &str, body: &str) -> (u16, String) {
    let req = Request { method: method.into(), path: path.into(), body: body.into(), ..Default::default() };
    match handle_api(&req) {
        Some(r) => (r.status, String::from_utf8(r.body).unwrap()),
        None => (404, String::new()),
    }
}

#[test]
fn routes_run_procs_with_json() {
    let r = call("POST", "/api/measure", r#"{"shapes": ["Dot", {"Circle": 1}, {"Rect": {"w": 2, "h": 3}}]}"#);
    assert_eq!(r, (200, r#"{"count":3,"total":9.141592653589793,"kinds":{"circle":1,"dot":1,"rect":1}}"#.into()));
    assert_eq!(call("GET", "/api/hello", r#"{"name": "x", "times": 1}"#), (200, r#""hello, x!""#.into()));
    let r = call("POST", "/api/measure", r#"{"shapes": [{"Rect": {"w": 2}}]}"#);
    assert_eq!(r, (400, r#"{"error":"parameter shapes: missing field h"}"#.into()));
    assert_eq!(call("GET", "/api/measure", "{}").0, 404);
}
