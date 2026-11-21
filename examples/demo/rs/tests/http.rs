//! The demo's HTTP routes ([http.api]), called without a network.

use vandorf_demo::app::http::{Value, handle_api};

fn call(method: &str, path: &str, params: &str) -> Option<Result<String, (u16, String)>> {
    handle_api(method, path, &Value::parse(params).unwrap())
}

#[test]
fn routes_run_procs_with_json() {
    let r = call("POST", "/api/measure", r#"{"shapes": ["Dot", {"Circle": 1}, {"Rect": {"w": 2, "h": 3}}]}"#).unwrap().unwrap();
    assert_eq!(r, r#"{"count":3,"total":9.141592653589793,"kinds":{"circle":1,"dot":1,"rect":1}}"#);
    assert_eq!(call("GET", "/api/hello", r#"{"name": "x", "times": 1}"#).unwrap().unwrap(), r#""hello, x!""#);
    let (status, msg) = call("POST", "/api/measure", r#"{"shapes": [{"Rect": {"w": 2}}]}"#).unwrap().unwrap_err();
    assert_eq!((status, msg.as_str()), (400, "parameter shapes: missing field h"));
    assert!(call("GET", "/api/measure", "{}").is_none());
}
