// The fixed part of a generated http.rs: JSON values and their parser, the
// Json trait for primitives, lists and maps, and a std-only HTTP server.
// Generated code adds Json impls for the program's types and a handler per
// server. Not compiled here: included as text by codegen::rust.
//
// Requests: a route's proc takes its parameters from a JSON object body,
// or from the query string (each value read as JSON if it parses, else as
// a string). Responses: 200 and the result as JSON; 400 for a request that
// doesn't decode; 500 and {"error": ..} when the proc fails.

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number as written.
    Num(String),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn parse(s: &str) -> Result<Value, String> {
        let mut p = Parser { b: s.as_bytes(), i: 0 };
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() {
            return Err(format!("unexpected input at byte {}", p.i));
        }
        Ok(v)
    }

    fn object(&self, what: &str) -> Result<&[(String, Value)], String> {
        match self {
            Value::Obj(fs) => Ok(fs),
            _ => Err(format!("expected an object for {what}")),
        }
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", c as char, self.i))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.ws();
        match self.b.get(self.i) {
            None => Err("unexpected end of input".into()),
            Some(b'n') => self.word("null", Value::Null),
            Some(b't') => self.word("true", Value::Bool(true)),
            Some(b'f') => self.word("false", Value::Bool(false)),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut xs = vec![];
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Value::Arr(xs));
                }
                loop {
                    xs.push(self.value()?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Value::Arr(xs));
                        }
                        _ => return Err(format!("expected ',' or ']' at byte {}", self.i)),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut fs = vec![];
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Value::Obj(fs));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    fs.push((k, self.value()?));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Obj(fs));
                        }
                        _ => return Err(format!("expected ',' or '}}' at byte {}", self.i)),
                    }
                }
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                let start = self.i;
                self.i += 1;
                while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') {
                    self.i += 1;
                }
                Ok(Value::Num(String::from_utf8_lossy(&self.b[start..self.i]).into_owned()))
            }
            Some(_) => Err(format!("unexpected character at byte {}", self.i)),
        }
    }

    fn word(&mut self, w: &str, v: Value) -> Result<Value, String> {
        if self.b[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(format!("unexpected input at byte {}", self.i))
        }
    }

    fn string(&mut self) -> Result<String, String> {
        if self.b.get(self.i) != Some(&b'"') {
            return Err(format!("expected a string at byte {}", self.i));
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            match self.b.get(self.i) {
                None => return Err("unterminated string".into()),
                Some(b'"') => {
                    self.i += 1;
                    return String::from_utf8(out).map_err(|e| e.to_string());
                }
                Some(b'\\') => {
                    let c = *self.b.get(self.i + 1).ok_or("unterminated string")?;
                    self.i += 2;
                    match c {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let hex = std::str::from_utf8(self.b.get(self.i..self.i + 4).ok_or("bad \\u escape")?).map_err(|e| e.to_string())?;
                            let cp = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                            self.i += 4;
                            let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                            out.extend_from_slice(ch.to_string().as_bytes());
                        }
                        c => out.push(c),
                    }
                }
                Some(&c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A value that crosses the HTTP boundary as JSON.
pub trait Json: Sized {
    fn to_json(&self, out: &mut String);
    fn from_json(v: &Value) -> Result<Self, String>;
}

impl Json for () {
    fn to_json(&self, out: &mut String) {
        out.push_str("null");
    }
    fn from_json(_: &Value) -> Result<Self, String> {
        Ok(())
    }
}

impl Json for i64 {
    fn to_json(&self, out: &mut String) {
        out.push_str(&self.to_string());
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        match v {
            Value::Num(n) => n.parse().map_err(|_| format!("{n} is not an int")),
            _ => Err("expected an int".into()),
        }
    }
}

impl Json for f64 {
    fn to_json(&self, out: &mut String) {
        if self.is_finite() {
            out.push_str(&format!("{self:?}"));
        } else {
            out.push_str("null");
        }
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        match v {
            Value::Num(n) => n.parse().map_err(|_| format!("{n} is not a float")),
            _ => Err("expected a float".into()),
        }
    }
}

impl Json for bool {
    fn to_json(&self, out: &mut String) {
        out.push_str(if *self { "true" } else { "false" });
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        match v {
            Value::Bool(b) => Ok(*b),
            _ => Err("expected a bool".into()),
        }
    }
}

impl Json for types::Str {
    fn to_json(&self, out: &mut String) {
        write_str(self, out);
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        match v {
            Value::Str(s) => Ok(types::Str::from(s.as_str())),
            _ => Err("expected a string".into()),
        }
    }
}

impl<T: Json> Json for std::sync::Arc<T> {
    fn to_json(&self, out: &mut String) {
        (**self).to_json(out);
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        Ok(std::sync::Arc::new(T::from_json(v)?))
    }
}

impl<T: Json + Clone> Json for types::List<T> {
    fn to_json(&self, out: &mut String) {
        out.push('[');
        for (i, x) in self.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            x.to_json(out);
        }
        out.push(']');
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        match v {
            Value::Arr(xs) => xs.iter().map(T::from_json).collect::<Result<Vec<_>, _>>().map(types::List::from),
            _ => Err("expected an array".into()),
        }
    }
}

/// A map key: maps are JSON objects, so keys are strings.
pub trait JsonKey: Sized + Ord {
    fn key(&self) -> String;
    fn from_key(k: &str) -> Result<Self, String>;
}

impl JsonKey for types::Str {
    fn key(&self) -> String {
        self.to_string()
    }
    fn from_key(k: &str) -> Result<Self, String> {
        Ok(k.into())
    }
}

impl JsonKey for i64 {
    fn key(&self) -> String {
        self.to_string()
    }
    fn from_key(k: &str) -> Result<Self, String> {
        k.parse().map_err(|_| format!("map key {k} is not an int"))
    }
}

impl<K: JsonKey + Clone, V: Json + Clone> Json for types::Map<K, V> {
    fn to_json(&self, out: &mut String) {
        out.push('{');
        for (i, (k, v)) in self.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_str(&k.key(), out);
            out.push(':');
            v.to_json(out);
        }
        out.push('}');
    }
    fn from_json(v: &Value) -> Result<Self, String> {
        let fs = v.object("a map")?;
        fs.iter().map(|(k, v)| Ok((K::from_key(k)?, V::from_json(v)?))).collect()
    }
}

fn field<'a>(fs: &'a [(String, Value)], name: &str) -> Result<&'a Value, String> {
    fs.iter().find(|(k, _)| k == name).map(|(_, v)| v).ok_or_else(|| format!("missing field {name}"))
}

/// A route's parameter, decoded; a failure is a 400.
fn arg<T: Json>(params: &Value, name: &str) -> Result<T, (u16, String)> {
    let fs = params.object("the parameters").map_err(|e| (400, e))?;
    let v = fs.iter().find(|(k, _)| k == name).map(|(_, v)| v).ok_or_else(|| (400, format!("missing parameter {name}")))?;
    T::from_json(v).map_err(|e| (400, format!("parameter {name}: {e}")))
}

fn json<T: Json>(x: &T) -> String {
    let mut out = String::new();
    x.to_json(&mut out);
    out
}

// ---- The server ----

/// A request: header names lowercased, the query string undecoded.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A server's routes: Some(the response) for a route it has.
pub type Handler = fn(&Request) -> Option<Response>;

/// A file served as it is: URL path, content type, contents.
pub type Static = (&'static str, &'static str, &'static str);

fn json_response(status: u16, body: String) -> Response {
    Response { status, headers: vec![("Content-Type".into(), "application/json".into())], body: body.into_bytes() }
}

fn error_response(status: u16, msg: &str) -> Response {
    json_response(status, json(&types::Map::from_iter([(types::Str::from("error"), types::Str::from(msg))])))
}

/// The query string's parameters, decoded.
fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

/// A JSON route: its parameters from a JSON object body, or the query
/// string (each value read as JSON if it parses, else as a string).
fn json_route(req: &Request, call: fn(&Value) -> Result<String, (u16, String)>) -> Response {
    let params = if req.body.trim().is_empty() {
        Value::Obj(query_pairs(&req.query).into_iter().map(|(k, v)| (k, Value::parse(&v).unwrap_or(Value::Str(v)))).collect())
    } else {
        match Value::parse(&req.body) {
            Ok(v) => v,
            Err(e) => return error_response(400, &format!("body: {e}")),
        }
    };
    match call(&params) {
        Ok(text) => json_response(200, text),
        Err((status, msg)) => error_response(status, &msg),
    }
}

/// A server's main: `[--addr host:port]` from the command line (else
/// `addr`, the manifest's), then serves.
pub fn run(addr: &str, handler: Handler, statics: &'static [Static]) {
    let name = std::env::args().next().map(|a| a.rsplit('/').next().unwrap_or("").to_string()).unwrap_or_default();
    let usage = format!("usage: {name} [--addr host:port]   (default {addr})");
    let mut addr = addr.to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match (a.as_str(), args.next()) {
            ("--addr", Some(v)) => addr = v,
            ("-h" | "--help", _) => {
                eprintln!("{usage}");
                return;
            }
            _ => {
                eprintln!("error: unexpected {a}\n{usage}");
                std::process::exit(2);
            }
        }
    }
    serve(&addr, handler, statics);
}

static ADDR: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The address being served on, once serving ("" before).
pub fn addr() -> &'static str {
    ADDR.get().map_or("", String::as_str)
}

/// Serves until the process is stopped, a thread per connection.
pub fn serve(addr: &str, handler: Handler, statics: &'static [Static]) {
    let listener = match std::net::TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: cannot listen on {addr}: {e}");
            std::process::exit(1);
        }
    };
    let _ = ADDR.set(addr.to_string());
    eprintln!("listening on http://{addr}");
    for stream in listener.incoming().flatten() {
        let _ = thread_builder().spawn(move || {
            let _ = connection(stream, handler, statics);
        });
    }
}

/// Unoptimized, a proc's frame holds every node's temporaries at once, so
/// grammar-deep recursion overflows the default 2 MiB; debug builds give
/// their threads more.
fn thread_builder() -> std::thread::Builder {
    let b = std::thread::Builder::new();
    #[cfg(debug_assertions)]
    let b = b.stack_size(64 << 20);
    b
}

fn connection(mut stream: std::net::TcpStream, handler: Handler, statics: &[Static]) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader, Read, Write};
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string());
    let mut headers = vec![];
    let mut length = 0;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            if k == "content-length" {
                length = v.parse().unwrap_or(0);
            }
            headers.push((k, v));
        }
    }
    let mut body = vec![0; length.min(64 << 20)];
    reader.read_exact(&mut body)?;
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let req = Request { method, path: path.into(), query: query.into(), headers, body: String::from_utf8_lossy(&body).into_owned() };
    let resp = respond(&req, handler, statics);
    let reason = match resp.status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let mut head = format!("HTTP/1.1 {} {reason}\r\n", resp.status);
    for (k, v) in &resp.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\nConnection: close\r\n\r\n", resp.body.len()));
    stream.write_all(head.as_bytes())?;
    stream.write_all(&resp.body)?;
    stream.flush()
}

fn respond(req: &Request, handler: Handler, statics: &[Static]) -> Response {
    if req.method == "GET"
        && let Some((_, ctype, text)) = statics.iter().find(|(p, _, _)| *p == req.path)
    {
        return Response { status: 200, headers: vec![("Content-Type".into(), ctype.to_string())], body: text.as_bytes().to_vec() };
    }
    handler(req).unwrap_or_else(|| error_response(404, &format!("no route {} {}", req.method, req.path)))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(x) => {
                    out.push(x);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
