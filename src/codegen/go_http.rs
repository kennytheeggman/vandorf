//! HTTP servers in Go, after the Rust backend's http.rs: http_vandorf.go
//! in the code package (the runtime, a JSON encoder and decoder for every
//! type, and a handler per server) and a package main per server. Each
//! server's static files go into its main package as strings, since
//! go:embed can't reach outside a package's directory.
//!
//! A JSON route's call is `httpCall<Proc>`, a raw route's `httpRaw<Proc>`;
//! a server with a setup has calls of its own (`httpCall<Server><Proc>`),
//! which pass its context as ctx. The main package calls `HTTPRun` (or
//! `HTTPRunSetup` with `HTTPSetup<Server>`) with `HTTPHandle<Server>`.

use super::rust::{RAW_REQUEST, RAW_RESPONSE, raw_shape, show_shape};
use super::types::{Shape, TyRef, TypeTable};
use super::{Artifact, Writer, exported, find_proc, gname, quote};
use crate::sema::{NodeId, Program};
use crate::types::{Prim, Type};
use std::collections::BTreeMap;

/// An HTTP server whose routes run top-level procs (as rust::ServerSpec).
#[derive(Debug, Clone, Default)]
pub struct GoServer {
    pub name: String,
    pub addr: String,
    /// A top-level proc run before serving, whose result every route and
    /// raw proc takes as `ctx`.
    pub setup: Option<String>,
    /// (method, path, proc): the proc takes its parameters and returns its
    /// result as JSON.
    pub routes: Vec<(String, String, String)>,
    /// Routes whose proc takes the whole request and returns the whole
    /// response.
    pub raw: Vec<(String, String, String)>,
    /// (URL path, file as the manifest names it, content type, contents).
    pub statics: Vec<(String, String, String, String)>,
    /// The main package's directory, relative to the code package's.
    pub path: String,
}

/// The fixed part of http_vandorf.go.
pub(crate) const HTTP_RUNTIME: &str = include_str!("http_runtime.go");

/// What http_vandorf.go imports, besides the types package.
pub(crate) const HTTP_IMPORTS: &[&str] =
    &["cmp", "context", "errors", "fmt", "io", "math", "net", "net/http", "os", "slices", "strconv", "strings", "unicode/utf8"];

/// A server's name as part of a Go identifier: `web-api` -> `Web_api`.
fn server_ident(name: &str) -> String {
    exported(name).chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// A Go string literal for any text: control characters escaped too.
pub(crate) fn go_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Http<'a> {
    prog: &'a Program,
    table: &'a TypeTable,
    /// Qualifier of named types in the code package: `types.` or empty.
    tq: &'a str,
}

impl Http<'_> {
    fn render(&self, r: &TyRef) -> String {
        match r {
            TyRef::Prim(p) => match p {
                Prim::Void => "struct{}",
                Prim::Int => "int",
                Prim::Float => "float64",
                Prim::Bool => "bool",
                Prim::String => "string",
            }
            .into(),
            TyRef::Array(e) => format!("[]{}", self.render(e)),
            TyRef::Map(k, v) => format!("map[{}]{}", self.render(k), self.render(v)),
            TyRef::Named(i) => format!("{}{}", self.tq, self.table.types[*i].name),
        }
    }

    fn go_type(&self, t: &Type) -> String {
        self.render(&self.table.get(t))
    }

    /// A func encoding a value of type r: `func(*strings.Builder, T)`.
    fn enc(&self, r: &TyRef) -> String {
        match r {
            TyRef::Prim(p) => format!("httpWrite{}", prim_name(*p)),
            TyRef::Array(e) => format!("httpListW({})", self.enc(e)),
            TyRef::Map(k, v) => format!("httpMapW(http{}Key, {})", key_name(k), self.enc(v)),
            TyRef::Named(i) => format!("httpEnc{}", self.table.types[*i].name),
        }
    }

    /// A func decoding a value of type r: `func(any) (T, error)`.
    fn dec(&self, r: &TyRef) -> String {
        match r {
            TyRef::Prim(p) => format!("httpRead{}", prim_name(*p)),
            TyRef::Array(e) => format!("httpListR({})", self.dec(e)),
            TyRef::Map(k, v) => format!("httpMapR(http{}FromKey, {})", key_name(k), self.dec(v)),
            TyRef::Named(i) => format!("httpDec{}", self.table.types[*i].name),
        }
    }

    /// An encoder and a decoder per named type.
    fn codecs(&self, w: &mut Writer) {
        for (i, t) in self.table.types.iter().enumerate() {
            let ty = format!("{}{}", self.tq, t.name);
            let boxed = |n: &str| self.table.is_boxed(i, n);
            match &t.shape {
                Shape::Struct(fs) => {
                    w.line(&format!("func httpEnc{}(b *strings.Builder, x {ty}) {{", t.name));
                    w.line("b.WriteByte('{')");
                    for (j, (n, r)) in fs.iter().enumerate() {
                        w.line(&format!("b.WriteString({})", quote(&format!("{}{}:", if j > 0 { "," } else { "" }, quote(n)))));
                        let v = if boxed(n) { format!("*x.{}", exported(n)) } else { format!("x.{}", exported(n)) };
                        w.line(&format!("{}(b, {v})", self.enc(r)));
                    }
                    w.line("b.WriteByte('}')");
                    w.line("}");
                    w.line("");
                    w.line(&format!("func httpDec{}(v any) ({ty}, error) {{", t.name));
                    w.line(&format!("var x {ty}"));
                    w.line(&format!("{}, err := httpObject(v, {})", if fs.is_empty() { "_" } else { "fs" }, quote(&t.name)));
                    w.line("if err != nil {");
                    w.line("return x, err");
                    w.line("}");
                    for (n, r) in fs {
                        w.line(&format!("if f, err := httpField(fs, {}); err != nil {{", quote(n)));
                        w.line("return x, err");
                        w.line(&format!("}} else if y, err := {}(f); err != nil {{", self.dec(r)));
                        w.line(&format!("return x, errors.New({} + err.Error())", quote(&format!("{n}: "))));
                        w.line("} else {");
                        w.line(&format!("x.{} = {}", exported(n), if boxed(n) { "&y" } else { "y" }));
                        w.line("}");
                    }
                    w.line("return x, nil");
                    w.line("}");
                    w.line("");
                }
                Shape::Enum(vs) => {
                    w.line(&format!("func httpEnc{}(b *strings.Builder, x {ty}) {{", t.name));
                    w.line("switch x.Tag {");
                    for (n, r) in vs {
                        w.line(&format!("case {}:", quote(n)));
                        if *r == TyRef::Prim(Prim::Void) {
                            w.line(&format!("b.WriteString({})", quote(&quote(n))));
                        } else {
                            w.line(&format!("b.WriteString({})", quote(&format!("{{{}:", quote(n)))));
                            let v = if boxed(n) { format!("*x.{}", exported(n)) } else { format!("x.{}", exported(n)) };
                            w.line(&format!("{}(b, {v})", self.enc(r)));
                            w.line("b.WriteByte('}')");
                        }
                    }
                    w.line("default:");
                    w.line("b.WriteString(\"null\")");
                    w.line("}");
                    w.line("}");
                    w.line("");
                    w.line(&format!("func httpDec{}(v any) ({ty}, error) {{", t.name));
                    w.line(&format!("var x {ty}"));
                    w.line("switch v := v.(type) {");
                    w.line("case string:");
                    w.line("switch v {");
                    for (n, r) in vs {
                        if *r == TyRef::Prim(Prim::Void) {
                            w.line(&format!("case {}:", quote(n)));
                            w.line(&format!("x.Tag = {}", quote(n)));
                            w.line("return x, nil");
                        }
                    }
                    w.line("}");
                    w.line(&format!("return x, fmt.Errorf({}, v)", quote(&format!("{} has no variant %s without a payload", t.name))));
                    w.line("case httpObj:");
                    w.line("if len(v) == 1 {");
                    w.line("switch v[0].Key {");
                    for (n, r) in vs {
                        if *r != TyRef::Prim(Prim::Void) {
                            w.line(&format!("case {}:", quote(n)));
                            w.line(&format!("y, err := {}(v[0].Val)", self.dec(r)));
                            w.line("if err != nil {");
                            w.line("return x, err");
                            w.line("}");
                            w.line(&format!("x.Tag = {}", quote(n)));
                            w.line(&format!("x.{} = {}", exported(n), if boxed(n) { "&y" } else { "y" }));
                            w.line("return x, nil");
                        }
                    }
                    w.line("}");
                    w.line(&format!("return x, fmt.Errorf({}, v[0].Key)", quote(&format!("{} has no variant %s with a payload", t.name))));
                    w.line("}");
                    w.line("}");
                    w.line(&format!(
                        "return x, errors.New({})",
                        quote(&format!("expected a variant of {}: \"Tag\" or {{\"Tag\": payload}}", t.name)),
                    ));
                    w.line("}");
                    w.line("");
                }
            }
        }
    }

    /// A JSON route's call: the request's parameters decoded, then the
    /// proc. With a setup (`ctx`: its static's name and the server), the
    /// context is its ctx parameter.
    fn call(&self, w: &mut Writer, fname: &str, pid: NodeId, p: &str, ctx: Option<(&str, &str)>) {
        let prog = self.prog;
        match ctx {
            Some((_, server)) => w.line(&format!("// {fname} runs proc {p} with the request's parameters and http server {server}'s context.")),
            None => w.line(&format!("// {fname} runs proc {p} with the request's parameters.")),
        }
        w.line(&format!("func {fname}(ctx context.Context, params any) (string, int, error) {{"));
        let mut args = vec!["ctx".to_string()];
        for (n, t) in prog.params(pid) {
            let var = format!("arg{}", exported(n));
            match ctx {
                Some((stat, server)) if n == "ctx" => {
                    w.line(&format!("if !{stat}Set {{"));
                    w.line(&format!("return \"\", 500, errors.New({})", quote(&format!("http server {server} is not set up"))));
                    w.line("}");
                    w.line(&format!("{var} := {stat}"));
                }
                _ => {
                    w.line(&format!("{var}, err := httpArg(params, {}, {})", quote(n), self.dec(&self.table.get(t))));
                    w.line("if err != nil {");
                    w.line("return \"\", 400, err");
                    w.line("}");
                }
            }
            args.push(var);
        }
        w.line(&format!("result, err := Run{}({})", gname(prog, pid), args.join(", ")));
        w.line("if err != nil {");
        w.line("return \"\", 500, err");
        w.line("}");
        w.line(&format!("return httpJSON({}, result), 200, nil", self.enc(&self.table.get(prog.ret(pid)))));
        w.line("}");
        w.line("");
    }

    /// A raw route's call: the request as the proc's request struct, and
    /// with a setup the context as ctx.
    fn raw(&self, w: &mut Writer, fname: &str, pid: NodeId, p: &str, req_ty: &Type, ctx: Option<(&str, &str)>) {
        let prog = self.prog;
        match ctx {
            Some((_, server)) => w.line(&format!("// {fname} runs proc {p} on the raw request with http server {server}'s context.")),
            None => w.line(&format!("// {fname} runs proc {p} on the raw request.")),
        }
        w.line(&format!("func {fname}(ctx context.Context, req *HTTPRequest) HTTPResponse {{"));
        w.line(&format!(
            "r := {}{{Method: req.Method, Path: req.Path, Query: httpQueryMap(req.Query), Headers: req.Headers, Body: req.Body}}",
            self.go_type(req_ty),
        ));
        let mut args = vec!["ctx".to_string()];
        if let Some((stat, server)) = ctx {
            w.line(&format!("if !{stat}Set {{"));
            w.line(&format!("return httpErrorResponse(500, {})", quote(&format!("http server {server} is not set up"))));
            w.line("}");
            args.extend(prog.params(pid).iter().map(|(n, _)| if n == "ctx" { stat.to_string() } else { "r".into() }));
        } else {
            args.push("r".into());
        }
        w.line(&format!("res, err := Run{}({})", gname(prog, pid), args.join(", ")));
        w.line("if err != nil {");
        w.line("return httpErrorResponse(500, err.Error())");
        w.line("}");
        w.line("return HTTPResponse{Status: res.Status, Headers: res.Headers, Body: res.Body}");
        w.line("}");
        w.line("");
    }

    /// A server with a setup proc: its context, HTTPSetup<Server> to run the
    /// setup before serving, and a call per route and raw route passing the
    /// context as ctx (never read from the request).
    fn setup_calls(&self, w: &mut Writer, srv: &GoServer, setup: &str) -> Result<(), String> {
        let prog = self.prog;
        let sid = find_proc(prog, setup)
            .ok_or_else(|| format!("http server {}'s setup runs proc {setup}, but there is no top-level proc {setup}", srv.name))?;
        if !prog.params(sid).is_empty() {
            return Err(format!("http server {}: setup proc {setup} must take no parameters", srv.name));
        }
        let (ctx_ty, ident) = (prog.ret(sid), server_ident(&srv.name));
        let (gctx, stat) = (self.go_type(ctx_ty), format!("httpCtx{ident}"));
        // Each of the server's procs must take ctx, of the setup's type.
        let check = |kind: &str, p: &str, pid: NodeId| -> Result<(), String> {
            match prog.params(pid).iter().find(|(n, _)| n == "ctx") {
                None => Err(format!("http server {}: {kind} proc {p} must take ctx: {ctx_ty}, its setup proc {setup}'s result", srv.name)),
                Some((_, t)) if self.go_type(t) != gctx => {
                    Err(format!("http server {}: {kind} proc {p} takes ctx: {t}, but its setup proc {setup} returns {ctx_ty}", srv.name))
                }
                Some(_) => Ok(()),
            }
        };
        w.line(&format!("// {stat} is http server {}'s context: proc {setup}'s result, set before serving.", srv.name));
        w.line(&format!("var {stat} {gctx}"));
        w.line(&format!("var {stat}Set bool"));
        w.line("");
        w.line(&format!("// HTTPSetup{ident} runs proc {setup}, the setup of http server {}.", srv.name));
        w.line(&format!("func HTTPSetup{ident}(ctx context.Context) error {{"));
        w.line(&format!("c, err := Run{}(ctx)", gname(prog, sid)));
        w.line("if err != nil {");
        w.line("return err");
        w.line("}");
        w.line(&format!("{stat}, {stat}Set = c, true"));
        w.line("return nil");
        w.line("}");
        w.line("");
        let mut procs: Vec<&str> = srv.routes.iter().map(|(_, _, p)| p.as_str()).collect();
        procs.sort();
        procs.dedup();
        for p in procs {
            let pid = find_proc(prog, p).ok_or_else(|| format!("http server {} runs proc {p}, but there is no top-level proc {p}", srv.name))?;
            check("route", p, pid)?;
            self.call(w, &format!("httpCall{ident}{}", gname(prog, pid)), pid, p, Some((&stat, &srv.name)));
        }
        let mut raws: Vec<&str> = srv.raw.iter().map(|(_, _, p)| p.as_str()).collect();
        raws.sort();
        raws.dedup();
        for p in raws {
            let pid = find_proc(prog, p).ok_or_else(|| format!("http server {} runs proc {p}, but there is no top-level proc {p}", srv.name))?;
            check("raw route", p, pid)?;
            let rest: Vec<&(String, Type)> = prog.params(pid).iter().filter(|(n, _)| n != "ctx").collect();
            if !(rest.len() == 1 && raw_shape(prog, &rest[0].1, RAW_REQUEST) && raw_shape(prog, prog.ret(pid), RAW_RESPONSE)) {
                return Err(format!(
                    "http server {}: raw route proc {p} must take ctx and one {}, and return {}",
                    srv.name, show_shape(RAW_REQUEST), show_shape(RAW_RESPONSE),
                ));
            }
            self.raw(w, &format!("httpRaw{ident}{}", gname(prog, pid)), pid, p, &rest[0].1, Some((&stat, &srv.name)));
        }
        Ok(())
    }
}

fn prim_name(p: Prim) -> &'static str {
    match p {
        Prim::Void => "Void",
        Prim::Int => "Int",
        Prim::Float => "Float",
        Prim::Bool => "Bool",
        Prim::String => "String",
    }
}

/// Map keys are ints or strings.
fn key_name(k: &TyRef) -> &'static str {
    match k {
        TyRef::Prim(Prim::Int) => "Int",
        _ => "String",
    }
}

/// http_vandorf.go's code: the runtime, every type's JSON, the calls, and
/// a handler per server.
pub(crate) fn http_body(prog: &Program, table: &TypeTable, tq: &str, servers: &[GoServer]) -> Result<String, String> {
    let h = Http { prog, table, tq };
    let mut w = Writer::new("\t");
    w.raw(HTTP_RUNTIME);
    w.line("");
    h.codecs(&mut w);
    // One call per proc a route runs (servers with a setup get their own,
    // below).
    let plain: Vec<&GoServer> = servers.iter().filter(|s| s.setup.is_none()).collect();
    let mut procs: Vec<&str> = plain.iter().flat_map(|s| s.routes.iter().map(|(_, _, p)| p.as_str())).collect();
    procs.sort();
    procs.dedup();
    for p in procs {
        let pid = find_proc(prog, p).ok_or_else(|| {
            let srv = plain.iter().find(|s| s.routes.iter().any(|r| r.2 == p)).unwrap();
            format!("http server {} runs proc {p}, but there is no top-level proc {p}", srv.name)
        })?;
        h.call(&mut w, &format!("httpCall{}", gname(prog, pid)), pid, p, None);
    }
    // One raw call per proc a raw route runs.
    let mut raws: Vec<&str> = plain.iter().flat_map(|s| s.raw.iter().map(|(_, _, p)| p.as_str())).collect();
    raws.sort();
    raws.dedup();
    for p in raws {
        let srv = plain.iter().find(|s| s.raw.iter().any(|r| r.2 == p)).unwrap();
        let pid = find_proc(prog, p).ok_or_else(|| format!("http server {} runs proc {p}, but there is no top-level proc {p}", srv.name))?;
        let params = prog.params(pid);
        let ok = params.len() == 1 && raw_shape(prog, &params[0].1, RAW_REQUEST) && raw_shape(prog, prog.ret(pid), RAW_RESPONSE);
        if !ok {
            return Err(format!(
                "http server {}: raw route proc {p} must take one {} and return {}",
                srv.name, show_shape(RAW_REQUEST), show_shape(RAW_RESPONSE),
            ));
        }
        h.raw(&mut w, &format!("httpRaw{}", gname(prog, pid)), pid, p, &params[0].1, None);
    }
    for srv in servers {
        if let Some(setup) = &srv.setup {
            h.setup_calls(&mut w, srv, setup)?;
        }
    }
    for srv in servers {
        let ident = server_ident(&srv.name);
        // A server with a setup has calls of its own.
        let own = if srv.setup.is_some() { ident.clone() } else { String::new() };
        w.line(&format!("// HTTPHandle{ident} routes the requests of http server {}.", srv.name));
        w.line(&format!("func HTTPHandle{ident}(ctx context.Context, req *HTTPRequest) (HTTPResponse, bool) {{"));
        w.line("switch req.Method + \" \" + req.Path {");
        // Two spellings of one route: the first is taken.
        let mut seen = std::collections::HashSet::new();
        for (m, path, p) in &srv.routes {
            if seen.insert(format!("{m} {path}")) {
                let pid = find_proc(prog, p).unwrap();
                w.line(&format!("case {}:", quote(&format!("{m} {path}"))));
                w.line(&format!("return httpJSONRoute(ctx, req, httpCall{own}{}), true", gname(prog, pid)));
            }
        }
        for (m, path, p) in &srv.raw {
            if seen.insert(format!("{m} {path}")) {
                let pid = find_proc(prog, p).unwrap();
                w.line(&format!("case {}:", quote(&format!("{m} {path}"))));
                w.line(&format!("return httpRaw{own}{}(ctx, req), true", gname(prog, pid)));
            }
        }
        w.line("}");
        w.line("return HTTPResponse{}, false");
        w.line("}");
        w.line("");
    }
    Ok(w.out)
}

/// A server's package main: its static files, then HTTPRun (or
/// HTTPRunSetup) with its handler. `header` writes a file's package clause
/// and imports.
pub(crate) fn server_main(
    srv: &GoServer,
    package: &str,
    import_base: Option<&str>,
    header: impl Fn(&str, &[(String, Option<String>)]) -> Writer,
) -> Result<Artifact, String> {
    let import = import_base.ok_or("the main package needs the code package's import path: pass --module or run inside a Go module")?;
    let alias = if package == "main" { "procs".to_string() } else { package.to_string() };
    let ident = server_ident(&srv.name);
    let mut w = header("main", &[(import.to_string(), Some(alias.clone()))]);
    let mut doc = vec![format!("// Command {} serves http server {} on {} (or --addr host:port):", srv.name, srv.name, srv.addr)];
    for (m, p, proc) in &srv.routes {
        doc.push(format!("//   - {m} {p} runs proc {proc}"));
    }
    for (m, p, proc) in &srv.raw {
        doc.push(format!("//   - {m} {p} runs proc {proc} on the raw request"));
    }
    for (p, f, _, _) in &srv.statics {
        doc.push(format!("//   - GET {p} serves {f}"));
    }
    if let Some(setup) = &srv.setup {
        doc.push("//".into());
        doc.push(format!("// Proc {setup} runs first; its result is each route's ctx."));
    }
    // The doc comment has to sit right above `package`, which header() wrote.
    w.out = w.out.replacen("package main", &format!("{}\npackage main", doc.join("\n")), 1);
    if srv.statics.is_empty() {
        w.line(&format!("var statics []{alias}.HTTPStatic"));
    } else {
        w.line(&format!("var statics = []{alias}.HTTPStatic{{"));
        for (p, _, ctype, text) in &srv.statics {
            w.line(&format!("{{Path: {}, Type: {}, Body: {}}},", quote(p), quote(ctype), go_quote(text)));
        }
        w.line("}");
    }
    w.line("");
    w.line("func main() {");
    match srv.setup {
        Some(_) => w.line(&format!("{alias}.HTTPRunSetup({}, {alias}.HTTPSetup{ident}, {alias}.HTTPHandle{ident}, statics)", quote(&srv.addr))),
        None => w.line(&format!("{alias}.HTTPRun({}, {alias}.HTTPHandle{ident}, statics)", quote(&srv.addr))),
    }
    w.line("}");
    Ok(Artifact { dir: srv.path.trim_matches('/').into(), files: BTreeMap::from([("main_vandorf.go".to_string(), w.out)]) })
}
