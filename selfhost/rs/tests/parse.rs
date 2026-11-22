//! vcc's parser (generated from ../sys/parse*.sys) against the reference
//! one (the vandorf crate's chumsky parser): both ASTs are printed as
//! S-expressions, spans included, and must match.
//!
//! The reference parser's spans include whitespace after the last token
//! (chumsky consumes it); vcc's end at the token. Reference spans are
//! trimmed before comparing.

use vandorf::ast as r;
use vcc::vcc::types as v;

struct Out<'a> {
    src: &'a str,
    s: String,
}

impl Out<'_> {
    fn open(&mut self, tag: &str) {
        self.s.push('(');
        self.s.push_str(tag);
    }
    fn close(&mut self) {
        self.s.push(')');
    }
    fn atom(&mut self, a: impl std::fmt::Display) {
        self.s.push(' ');
        self.s.push_str(&a.to_string());
    }
    /// A span as `@at:end`, trimming trailing whitespace and comments.
    fn span(&mut self, at: usize, end: usize) {
        let mut end = end;
        loop {
            let text = &self.src[at..end];
            let trimmed = text.trim_end();
            // A trailing `// comment` line.
            let cut = match trimmed.rfind('\n') {
                Some(nl) if trimmed[nl..].trim_start().starts_with("//") => nl,
                None if trimmed.trim_start().starts_with("//") => 0,
                _ => trimmed.len(),
            };
            let new = at + text[..cut].trim_end().len();
            if new == end {
                break;
            }
            end = new;
        }
        self.atom(format!("@{at}:{end}"));
    }
}

// ---- Reference AST ----

fn r_file(o: &mut Out, f: &r::File) {
    o.open("file");
    for t in &f.types {
        o.open("type");
        r_ident(o, &t.name);
        for p in &t.params {
            r_ident(o, p);
        }
        r_ty(o, &t.ty);
        o.close();
    }
    for i in &f.interfaces {
        o.open("interface");
        r_ident(o, &i.name);
        for f in &i.funcs {
            r_func(o, f);
        }
        o.close();
    }
    for n in &f.nodes {
        r_node(o, n);
    }
    o.close();
}

fn r_ident(o: &mut Out, i: &r::Ident) {
    o.atom(&i.name);
    o.span(i.span.start, i.span.end);
}

fn r_path(o: &mut Out, p: &r::Path) {
    o.open("path");
    for s in &p.segs {
        r_ident(o, s);
    }
    o.span(p.span.start, p.span.end);
    o.close();
}

fn r_node(o: &mut Out, n: &r::Node) {
    match n {
        r::Node::Func(f) => r_func(o, f),
        r::Node::Proc(p) => r_proc(o, p),
    }
}

fn r_func(o: &mut Out, f: &r::Func) {
    o.open("func");
    r_ident(o, &f.name);
    for p in &f.params {
        r_field(o, p);
    }
    match &f.ret {
        Some(t) => r_ty(o, t),
        None => o.atom("void"),
    }
    for i in &f.imps {
        o.open("impl");
        o.atom(&i.written);
        o.atom(&i.function);
        o.span(i.span.start, i.span.end);
        o.close();
    }
    o.close();
}

fn r_proc(o: &mut Out, p: &r::Proc) {
    o.open("proc");
    r_ident(o, &p.name);
    r_ident(o, &p.entry);
    r_ty(o, &p.ret);
    for n in &p.nodes {
        r_node(o, n);
    }
    for e in &p.edges {
        r_edge(o, e);
    }
    o.close();
}

fn r_field(o: &mut Out, f: &r::Field) {
    o.open("field");
    r_ident(o, &f.name);
    r_ty(o, &f.ty);
    o.close();
}

fn r_ty(o: &mut Out, t: &r::TypeExpr) {
    use r::TypeExprKind as K;
    match &t.kind {
        K::Named(i) => {
            o.open("named");
            r_ident(o, i);
        }
        K::Inst(i, args) => {
            o.open("inst");
            r_ident(o, i);
            for a in args {
                r_ty(o, a);
            }
        }
        K::Array(e) => {
            o.open("array");
            r_ty(o, e);
        }
        K::Map(k, v) => {
            o.open("map");
            r_ty(o, k);
            r_ty(o, v);
        }
        K::Struct(fs) => {
            o.open("struct");
            for f in fs {
                r_field(o, f);
            }
        }
        K::Enum(vs) => {
            o.open("enum");
            for v in vs {
                o.open("variant");
                r_ident(o, &v.name);
                if let Some(p) = &v.payload {
                    r_ty(o, p);
                }
                o.close();
            }
        }
    }
    o.span(t.span.start, t.span.end);
    o.close();
}

fn r_edge(o: &mut Out, e: &r::Edge) {
    o.open("edge");
    r_path(o, &e.source);
    match &e.kind {
        r::EdgeKind::Call(t) => {
            o.open("call");
            r_target(o, t);
        }
        r::EdgeKind::Match(arms) => {
            o.open("match");
            for a in arms {
                r_arm(o, a);
            }
        }
        r::EdgeKind::Collect { dim, target } => {
            o.open("collect");
            r_ident(o, dim);
            r_target(o, target);
        }
        r::EdgeKind::ForEach { dim, target } => {
            o.open("foreach");
            r_value(o, dim);
            r_target(o, target);
        }
    }
    o.close();
    o.span(e.span.start, e.span.end);
    o.close();
}

fn r_target(o: &mut Out, t: &r::Target) {
    o.open("target");
    o.s.push_str(&format!(" text={:?}", t.text));
    r_path(o, &t.node);
    for a in &t.args {
        o.open("arg");
        r_ident(o, &a.name);
        r_value(o, &a.value);
        o.close();
    }
    if let Some(a) = &t.alias {
        o.open("as");
        r_ident(o, a);
        o.close();
    }
    if let Some(v) = &t.id {
        o.open("id");
        r_value(o, v);
        o.close();
    }
    o.close();
}

fn r_arm(o: &mut Out, a: &r::Arm) {
    o.open("arm");
    match &a.head {
        r::Head::Pat(p) => r_pat(o, p),
        r::Head::Cond(c) => {
            o.open("cond");
            r_value(o, c);
            o.close();
        }
        r::Head::Else => o.atom("else"),
    }
    o.span(a.span.start, a.span.end);
    match &a.body {
        r::Body::Call(t) => r_target(o, t),
        r::Body::Switch(arms, span) => {
            o.open("switch");
            for a in arms {
                r_arm(o, a);
            }
            o.span(span.start, span.end);
            o.close();
        }
    }
    o.close();
}

fn r_pat(o: &mut Out, p: &r::Pat) {
    use r::PatKind as K;
    match &p.kind {
        K::Wild => o.open("_"),
        K::Bind(i) => {
            o.open("bind");
            r_ident(o, i);
        }
        K::Variant { tag, payload } => {
            o.open("variant");
            r_ident(o, tag);
            if let Some(q) = payload {
                r_pat(o, q);
            }
        }
        K::Struct(fs) => {
            o.open("struct");
            for (n, q) in fs {
                r_ident(o, n);
                r_pat(o, q);
            }
        }
        K::Or(alts) => {
            o.open("or");
            for a in alts {
                r_pat(o, a);
            }
        }
    }
    o.span(p.span.start, p.span.end);
    o.close();
}

fn r_value(o: &mut Out, v: &r::Value) {
    use r::ValueKind as K;
    match &v.kind {
        K::Ref(p) => {
            o.open("ref");
            r_path(o, p);
        }
        K::Int(i) => {
            o.open("int");
            o.atom(i);
        }
        K::Float(f) => {
            o.open("float");
            o.atom(f);
        }
        K::Str(s) => {
            o.open("str");
            o.atom(format!("{s:?}"));
        }
        K::Bool(b) => {
            o.open("bool");
            o.atom(b);
        }
        K::Apply { name, args } => {
            o.open("apply");
            r_path(o, name);
            for a in args {
                o.open("arg");
                if let Some(n) = &a.name {
                    r_ident(o, n);
                }
                r_value(o, &a.value);
                o.close();
            }
        }
        K::Struct(args) => {
            o.open("struct");
            for a in args {
                r_ident(o, &a.name);
                r_value(o, &a.value);
            }
        }
        K::Update { base, args } => {
            o.open("update");
            r_value(o, base);
            for a in args {
                r_ident(o, &a.name);
                r_value(o, &a.value);
            }
        }
        K::Array(es) => {
            o.open("array");
            for e in es {
                r_value(o, e);
            }
        }
        K::Map(kvs) => {
            o.open("map");
            for (k, v) in kvs {
                r_value(o, k);
                r_value(o, v);
            }
        }
        K::Unary(op, e) => {
            o.open("unary");
            o.atom(if *op == r::UnOp::Not { "!" } else { "-" });
            r_value(o, e);
        }
        K::Binary(op, l, rr) => {
            o.open("binary");
            o.atom(op.symbol());
            r_value(o, l);
            r_value(o, rr);
        }
        K::Index(e, k) => {
            o.open("index");
            r_value(o, e);
            r_value(o, k);
        }
        K::Field(e, n) => {
            o.open("field");
            r_value(o, e);
            r_ident(o, n);
        }
        K::In(k, m) => {
            o.open("in");
            r_value(o, k);
            r_value(o, m);
        }
    }
    o.span(v.span.start, v.span.end);
    o.close();
}

// ---- vcc's AST ----

fn sp(o: &mut Out, s: &v::Span) {
    o.span(s.at as usize, s.end as usize);
}

fn v_file(o: &mut Out, f: &v::File) {
    o.open("file");
    for t in &f.types {
        o.open("type");
        v_ident(o, &t.name);
        for p in &t.params {
            v_ident(o, p);
        }
        v_ty(o, &t.ty);
        o.close();
    }
    for i in &f.interfaces {
        o.open("interface");
        v_ident(o, &i.name);
        for f in &i.funcs {
            v_func(o, f);
        }
        o.close();
    }
    for n in &f.nodes {
        v_node(o, n);
    }
    o.close();
}

fn v_ident(o: &mut Out, i: &v::Ident) {
    o.atom(&i.name);
    sp(o, &i.span);
}

fn v_path(o: &mut Out, p: &v::Path) {
    o.open("path");
    for s in &p.segs {
        v_ident(o, s);
    }
    sp(o, &p.span);
    o.close();
}

fn v_node(o: &mut Out, n: &v::Node) {
    match n {
        v::Node::Func(f) => v_func(o, f),
        v::Node::Proc(p) => v_proc(o, p),
    }
}

fn v_func(o: &mut Out, f: &v::Func) {
    o.open("func");
    v_ident(o, &f.name);
    for p in &f.params {
        v_field(o, p);
    }
    match &f.ret {
        v::OptType::Some(t) => v_ty(o, t),
        v::OptType::None => o.atom("void"),
    }
    for i in &f.imps {
        o.open("impl");
        o.atom(&i.file);
        o.atom(&i.function);
        sp(o, &i.span);
        o.close();
    }
    o.close();
}

fn v_proc(o: &mut Out, p: &v::Proc) {
    o.open("proc");
    v_ident(o, &p.name);
    v_ident(o, &p.entry);
    v_ty(o, &p.ret);
    for n in &p.nodes {
        v_node(o, n);
    }
    for e in &p.edges {
        v_edge(o, e);
    }
    o.close();
}

fn v_field(o: &mut Out, f: &v::Field) {
    o.open("field");
    v_ident(o, &f.name);
    v_ty(o, &f.ty);
    o.close();
}

fn v_ty(o: &mut Out, t: &v::TypeExpr) {
    use v::TypeKind as K;
    match &*t.kind {
        K::Named(i) => {
            o.open("named");
            v_ident(o, i);
        }
        K::Inst(i) => {
            o.open("inst");
            v_ident(o, &i.name);
            for a in &i.args {
                v_ty(o, a);
            }
        }
        K::Array(e) => {
            o.open("array");
            v_ty(o, e);
        }
        K::Map(m) => {
            o.open("map");
            v_ty(o, &m.key);
            v_ty(o, &m.value);
        }
        K::Struct(fs) => {
            o.open("struct");
            for f in fs {
                v_field(o, f);
            }
        }
        K::Enum(vs) => {
            o.open("enum");
            for var in vs {
                o.open("variant");
                v_ident(o, &var.name);
                if let v::OptType::Some(p) = &var.payload {
                    v_ty(o, p);
                }
                o.close();
            }
        }
    }
    sp(o, &t.span);
    o.close();
}

fn v_edge(o: &mut Out, e: &v::Edge) {
    o.open("edge");
    v_path(o, &e.source);
    match &e.kind {
        v::EdgeKind::Call(t) => {
            o.open("call");
            v_target(o, t);
        }
        v::EdgeKind::Match(arms) => {
            o.open("match");
            for a in arms {
                v_arm(o, a);
            }
        }
        v::EdgeKind::Collect(c) => {
            o.open("collect");
            v_ident(o, &c.dim);
            v_target(o, &c.target);
        }
        v::EdgeKind::ForEach(f) => {
            o.open("foreach");
            v_value(o, &f.dim);
            v_target(o, &f.target);
        }
    }
    o.close();
    sp(o, &e.span);
    o.close();
}

fn v_target(o: &mut Out, t: &v::Target) {
    o.open("target");
    let text = o.src.get(t.inner.at as usize..t.inner.end as usize).unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
    o.s.push_str(&format!(" text={text:?}"));
    v_path(o, &t.node);
    for a in &t.args {
        o.open("arg");
        v_ident(o, &a.name);
        v_value(o, &a.value);
        o.close();
    }
    if let v::OptIdent::Some(a) = &t.alias {
        o.open("as");
        v_ident(o, a);
        o.close();
    }
    if let v::OptValue::Some(val) = &t.id {
        o.open("id");
        v_value(o, val);
        o.close();
    }
    o.close();
}

fn v_arm(o: &mut Out, a: &v::Arm) {
    o.open("arm");
    match &a.head {
        v::Head::Pat(p) => v_pat(o, p),
        v::Head::Cond(c) => {
            o.open("cond");
            v_value(o, c);
            o.close();
        }
        v::Head::Else => o.atom("else"),
    }
    sp(o, &a.span);
    match &a.body {
        v::Body::Call(t) => v_target(o, t),
        v::Body::Switch(s) => {
            o.open("switch");
            for a in &s.arms {
                v_arm(o, a);
            }
            sp(o, &s.span);
            o.close();
        }
    }
    o.close();
}

fn v_pat(o: &mut Out, p: &v::Pat) {
    use v::PatKind as K;
    match &*p.kind {
        K::Wild => o.open("_"),
        K::Bind(i) => {
            o.open("bind");
            v_ident(o, i);
        }
        K::Variant(var) => {
            o.open("variant");
            v_ident(o, &var.tag);
            if let v::OptPat::Some(q) = &*var.payload {
                v_pat(o, q);
            }
        }
        K::Struct(fs) => {
            o.open("struct");
            for f in fs {
                v_ident(o, &f.name);
                v_pat(o, &f.pat);
            }
        }
        K::Or(alts) => {
            o.open("or");
            for a in alts {
                v_pat(o, a);
            }
        }
    }
    sp(o, &p.span);
    o.close();
}

fn v_value(o: &mut Out, val: &v::Value) {
    use v::ValueKind as K;
    match &*val.kind {
        K::Ref(p) => {
            o.open("ref");
            v_path(o, p);
        }
        K::Int(i) => {
            o.open("int");
            o.atom(i);
        }
        K::Float(f) => {
            o.open("float");
            o.atom(f);
        }
        K::Str(s) => {
            o.open("str");
            o.atom(format!("{s:?}"));
        }
        K::Bool(b) => {
            o.open("bool");
            o.atom(b);
        }
        K::Apply(a) => {
            o.open("apply");
            v_path(o, &a.name);
            for arg in &a.args {
                o.open("arg");
                if let v::OptIdent::Some(n) = &arg.name {
                    v_ident(o, n);
                }
                v_value(o, &arg.value);
                o.close();
            }
        }
        K::Struct(args) => {
            o.open("struct");
            for a in args {
                v_ident(o, &a.name);
                v_value(o, &a.value);
            }
        }
        K::Update(u) => {
            o.open("update");
            v_value(o, &u.base);
            for a in u.args.iter() {
                v_ident(o, &a.name);
                v_value(o, &a.value);
            }
        }
        K::Array(es) => {
            o.open("array");
            for e in es {
                v_value(o, e);
            }
        }
        K::Map(kvs) => {
            o.open("map");
            for e in kvs {
                v_value(o, &e.key);
                v_value(o, &e.value);
            }
        }
        K::Unary(u) => {
            o.open("unary");
            o.atom(&u.op);
            v_value(o, &u.e);
        }
        K::Binary(b) => {
            o.open("binary");
            o.atom(&b.op);
            v_value(o, &b.l);
            v_value(o, &b.r);
        }
        K::Index(i) => {
            o.open("index");
            v_value(o, &i.e);
            v_value(o, &i.key);
        }
        K::Field(f) => {
            o.open("field");
            v_value(o, &f.e);
            v_ident(o, &f.name);
        }
        K::In(i) => {
            o.open("in");
            v_value(o, &i.key);
            v_value(o, &i.map);
        }
    }
    sp(o, &val.span);
    o.close();
}

// ---- The comparison ----

fn both(src: &str) -> (String, String) {
    let reference = vandorf::parser::parse(src, 0).unwrap_or_else(|e| panic!("reference parser: {e:?}"));
    let mine = vcc::vcc::run_parse(src.into()).unwrap_or_else(|e| panic!("vcc: {e}"));
    let mut a = Out { src, s: String::new() };
    r_file(&mut a, &reference);
    let mut b = Out { src, s: String::new() };
    v_file(&mut b, &mine);
    (a.s, b.s)
}

/// The first point where `a` and `b` differ, with context.
fn diff(a: &str, b: &str) -> String {
    let i = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let from = i.saturating_sub(120);
    format!("reference: ...{}\nvcc:       ...{}", &a[from..(i + 80).min(a.len())], &b[from..(i + 80).min(b.len())])
}

fn sys_files() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = vec![];
    for dir in ["examples/demo/sys", "selfhost/sys", "tests/fixtures"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "sys") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn parses_every_sys_file_like_the_reference() {
    let files = sys_files();
    assert!(files.len() > 10);
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        let (a, b) = both(&src);
        assert!(a == b, "{}:\n{}", f.display(), diff(&a, &b));
    }
}

#[test]
fn parses_tricky_syntax_like_the_reference() {
    for src in [
        // Precedence, prefix operators and negative literals, postfix chains.
        r#"proc P(A): int { A switch {
            (a || b && !c == d) -> X(i = -3, f = -1.5, g = - x),
            (1 + 2 * -x % 3 - y >= F(n = this.l, m = 2) + 1) -> X(),
            (k in this.m && this.m[k].x + 1 > [:].len && G(this)[0].y) -> X(),
            else -> id({ a = [1, 2,], b = ["k": 1, "j": 2,], c = S(), d = T({ x = this, type = [] }) })
        }; }"#,
        // Patterns and nested switches.
        r#"proc P(A): int { A switch {
            Lit(n) -> switch { (n > 0) -> X(), else -> Y() as Z },
            Neg(Lit(_)) | Add({ l = _, r = Neg(e) }) -> X(),
            Unit() -> X(),
            other -> id(0)
        }; }"#,
        // Types, impls, interfaces, nested procs, collect and for each.
        r#"proc P(A): int { func A(): int; A -> B(s = { ..this }, t = { ..A.x, y = 1, z = { ..q } }, u = { ..this, }); }"#,
        r#"type R<T> = { x: T, pos: int };
        type Pair<A, B> = { l: A, r: map[string]R<B> };
        type L = R<Pair<[]int, R<string>>>;
        func F(p: Pair<int, int>): R<L> = "a.rs:f";"#,
        r#"type T = enum { Leaf(int), Node({ type: string, kids: []T, m: map[string]T }) };
        interface Fs { func Read(path: string): string = "a.rs:read", "b.go:Read"; }
        func Top(x: int);
        proc Outer(A): int {
            func A(): []int;
            func B(i: int): int;
            proc Inner(C): int { func C(): int; };
            A for each (this) -> B(i = this);
            B collect each (B) -> Inner();
        };"#,
    ] {
        let (a, b) = both(src);
        assert!(a == b, "{src}\n{}", diff(&a, &b));
    }
}

#[test]
fn reports_syntax_errors() {
    for (src, want) in [
        ("proc P(A): int { func A(): int; A -> ; }", "expected a name, found ';'"),
        ("proc P(A) int {}", "expected : and the proc's type, found 'int'"),
        ("func F(x: int) = \"nocolon\";", "expected \"file:function\""),
        ("type T = enum { A, B(int };", "expected ) after the payload, found '}'"),
        ("proc P(A): int { A switch { (this) -> X() X() }; }", "expected , or }, found 'X'"),
        ("type R<T>= { x: T };", "write `> =` here: `>=` is the comparison operator"),
        ("type R<T> = { x: T }; func F(): R<int>= \"a.rs:f\";", "write `> =` here"),
        ("type A = R<int;", "expected > to close the type arguments, found ';'"),
        ("type R<> = int;", "expected a type parameter, found '>'"),
    ] {
        let e = vcc::vcc::run_parse(src.into()).unwrap_err().to_string();
        assert!(e.contains(want), "{src}: {e}");
    }
}
