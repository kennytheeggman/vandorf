//! vcc's parser (generated from ../sys/parse*.sys): ASTs printed as
//! S-expressions, spans included, against their golden files (first
//! recorded from the reference parser's, whose spans were trimmed of
//! trailing whitespace to match).

mod common;

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
    o.open("params");
    for f in &p.params {
        v_field(o, f);
    }
    o.close();
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
        v::EdgeKind::Spawn(sp) => {
            o.open("spawn");
            for b in sp.branches.iter() {
                o.open(if b.detach { "detach" } else { "branch" });
                v_target(o, &b.target);
                o.close();
            }
            o.open("join");
            v_target(o, &sp.join);
            o.close();
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

/// vcc's AST of src as an S-expression, spans included.
fn ast(src: &str) -> String {
    let mine = vcc::vcc::run_parse(src.into()).unwrap_or_else(|e| panic!("vcc: {e}"));
    let mut b = Out { src, s: String::new() };
    v_file(&mut b, &mine);
    b.s
}

fn sys_files() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = vec![];
    for dir in ["examples/demo/sys", "src"] {
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
fn parses_every_sys_file() {
    let files = sys_files();
    assert!(files.len() > 10);
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        if let Err(e) = vcc::vcc::run_parse(src.as_str().into()) {
            panic!("{}: {e}", f.display());
        }
    }
}

#[test]
fn parses_corpus() {
    for (name, src) in common::corpus() {
        if vcc::vcc::run_parse(src.as_str().into()).is_err() {
            assert!(!common::has("parse", &name), "{name} has a golden AST but no longer parses");
            continue;
        }
        common::check("parse", &name, &ast(&src));
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/demo/sys");
    let mut demo: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "sys")).collect();
    demo.sort();
    for f in demo {
        let src = std::fs::read_to_string(&f).unwrap();
        common::check("parse", &format!("demo-{}", f.file_stem().unwrap().to_string_lossy()), &ast(&src));
    }
}

#[test]
fn parses_tricky_syntax() {
    for (i, src) in [
        // Precedence, prefix operators and negative literals, postfix chains.
        r#"proc P(): int { P switch {
            (a || b && !c == d) -> X(i = -3, f = -1.5, g = - x),
            (1 + 2 * -x % 3 - y >= F(n = this.l, m = 2) + 1) -> X(),
            (k in this.m && this.m[k].x + 1 > [:].len && G(this)[0].y) -> X(),
            else -> id({ a = [1, 2,], b = ["k": 1, "j": 2,], c = S(), d = T({ x = this, type = [] }) })
        }; }"#,
        // Patterns and nested switches.
        r#"proc P(): int { P switch {
            Lit(n) -> switch { (n > 0) -> X(), else -> Y() as Z },
            Neg(Lit(_)) | Add({ l = _, r = Neg(e) }) -> X(),
            Unit() -> X(),
            other -> id(0)
        }; }"#,
        // Spawns: detached and aliased branches, a trailing comma, any join.
        r#"proc P(): int { P -> spawn { B(n = this), detach C(s = "x") as C2, D(), } join E(b = this.B); E -> spawn { detach F() } join id(1); }"#,
        // Types, impls, interfaces, nested procs, collect and for each.
        r#"proc P(): int { func A(): int; P -> B(s = { ..this }, t = { ..A.x, y = 1, z = { ..q } }, u = { ..this, }); }"#,
        r#"type R<T> = { x: T, pos: int };
        type Pair<A, B> = { l: A, r: map[string]R<B> };
        type L = R<Pair<[]int, R<string>>>;
        func F(p: Pair<int, int>): R<L> = "a.rs:f";"#,
        r#"type T = enum { Leaf(int), Node({ type: string, kids: []T, m: map[string]T }) };
        interface Fs { func Read(path: string): string = "a.rs:read", "b.go:Read"; }
        func Top(x: int);
        proc Outer(): int {
            func A(): []int;
            func B(i: int): int;
            proc Inner(): int { func C(): int; Inner -> C(); };
            Outer -> A();
            A for each (this) -> B(i = this);
            B collect each (B) -> Inner();
        };"#,
    ].iter().enumerate() {
        common::check("parse-tricky", &format!("c{i}"), &ast(src));
    }
}

#[test]
fn reports_syntax_errors() {
    for (src, want) in [
        ("proc P(): int { func A(): int; P -> ; }", "expected a call: a name, or id(value), found ';'"),
        ("proc P() int {}", "expected : and the proc's type, found 'int'"),
        ("func F(x: int) = \"nocolon\";", "expected \"file:function\""),
        ("type T = enum { A, B(int };", "expected ) after the payload, found '}'"),
        ("proc P(): int { P switch { (this) -> X() X() }; }", "expected , or }, found 'X'"),
        ("type R<T>= { x: T };", "write `> =` here: `>=` is the comparison operator"),
        ("type R<T> = { x: T }; func F(): R<int>= \"a.rs:f\";", "write `> =` here"),
        ("type A = R<int;", "expected > to close the type arguments, found ';'"),
        ("type R<> = int;", "expected a type parameter, found '>'"),
        // A proc with params starts its body from its own name.
        ("proc P(a: int): int { func F(x: int): int; F -> id(this); P -> F(x = this.a); }", "43:44: the body of proc P starts from P: write `P -> ..` first, found F"),
        ("proc P(A): int {}", "expected : after the field name, found ')'"),
    ] {
        let e = vcc::vcc::run_parse(src.into()).unwrap_err().to_string();
        assert!(e.contains(want), "{src}: {e}");
    }
}
