use vandorf::document::{self, EdgeType, Grid};

fn compile(src: &str) -> vandorf::sema::Program {
    vandorf::compile("test.sys", src).unwrap_or_else(|e| panic!("{e}"))
}

fn compile_err(src: &str) -> String {
    match vandorf::compile("test.sys", src) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    }
}

fn labels(g: &Grid) -> Vec<Vec<&str>> {
    g.rows.iter().map(|r| r.nodes.iter().map(|n| n.label.as_str()).collect()).collect()
}

fn pos(g: &Grid, label: &str) -> [usize; 2] {
    for (r, row) in g.rows.iter().enumerate() {
        for (c, n) in row.nodes.iter().enumerate() {
            if n.label == label {
                return [r, c];
            }
        }
    }
    panic!("{label} not in grid");
}

/// The fixture from the Go implementation, unchanged apart from having no impls.
const GO_FIXTURE: &str = include_str!("fixtures/test1.sys");

#[test]
fn go_fixture_compiles_and_lays_out() {
    let doc = document::generate(&compile(GO_FIXTURE));
    let bar = &doc.grids["Bar"];
    // End is first reached from Branch1 (row 4) and pushed below Branch2End.
    assert!(pos(bar, "End")[0] > pos(bar, "Branch2End")[0]);
    assert!(pos(bar, "End")[0] > pos(bar, "Branch1")[0]);
    let fe = &doc.grids["ForEach"];
    assert_eq!(labels(fe), vec![vec!["Start"], vec!["Process"], vec!["Collect"]]);
    let types: Vec<EdgeType> = fe.edges.iter().map(|e| e.ty).collect();
    assert_eq!(types, vec![EdgeType::ForEachExpand, EdgeType::ForEachCollect]);
}

#[test]
fn back_edge_does_not_push_down() {
    let doc = document::generate(&compile(
        r#"
        proc P(A): int {
            func A(i: int): int;
            func B(i: int): int;
            func C(i: int): int;
            A -> B(i = this);
            B -> C(i = this);
            C switch { (this < 3) -> A(i = this), else -> D(i = this) };
            func D(i: int): int;
        }
        "#,
    ));
    let g = &doc.grids["P"];
    assert_eq!(labels(g), vec![vec!["A"], vec!["B"], vec!["C"], vec!["D"]]);
    let back = g.edges.iter().find(|e| e.label.as_deref() == Some("this < 3")).unwrap();
    assert_eq!((back.source, back.target), ([2, 0], [0, 0]));
}

#[test]
fn diamond_in_same_row_and_pushdown_of_subtree() {
    // A -> {B, C}; B -> D; C -> X -> D. D (and its subtree E) must end below X.
    let doc = document::generate(&compile(
        r#"
        proc P(A): int {
            func A(): enum { L, R };
            func B(): int;
            func C(): int;
            func X(): int;
            func D(): int;
            func E(): int;
            A switch { L() -> B(), R() -> C() };
            B -> D();
            C -> X();
            X -> D();
            D -> E();
        }
        "#,
    ));
    let g = &doc.grids["P"];
    assert_eq!(
        labels(g),
        vec![vec!["A"], vec!["B", "C"], vec!["X"], vec!["D"], vec!["E"]]
    );
    // Every edge resolves to a placed node and points down except none here.
    for e in &g.edges {
        assert!(e.target[0] > e.source[0], "{e:?}");
    }
}

#[test]
fn continue_edges_inside_for_each() {
    let doc = document::generate(&compile(
        r#"
        proc P(A): int {
            func A(): []int;
            func B(i: int): int;
            func C(i: int): int;
            func D(l: []int): int;
            A for each (A) -> B(i = this);
            B -> C(i = this);
            C collect each (C) -> D(l = this);
        }
        "#,
    ));
    let types: Vec<EdgeType> = doc.grids["P"].edges.iter().map(|e| e.ty).collect();
    assert_eq!(types, vec![EdgeType::ForEachExpand, EdgeType::ForEachContinue, EdgeType::ForEachCollect]);
}

#[test]
fn impl_ref_becomes_sublabel() {
    let doc = document::generate(&compile(
        r#"proc P(A): int { func A(): int = "a/b.go:DoA"; }"#,
    ));
    assert_eq!(doc.grids["P"].rows[0].nodes[0].sublabel.as_deref(), Some("a/b.go:DoA"));
}

#[test]
fn nested_procs_get_their_own_grid() {
    let doc = document::generate(&compile(
        r#"
        proc Outer(Inner): int {
            proc Inner(A): int { func A(): int; };
        }
        "#,
    ));
    assert_eq!(doc.grids["Outer"].rows[0].nodes[0].label, "Outer.Inner");
    assert!(doc.grids.contains_key("Outer.Inner"));
}

#[test]
fn type_errors() {
    let e = compile_err(r#"proc P(A): int { func A(): string; func B(i: int): int; A -> B(i = this); }"#);
    assert!(e.contains("argument i of P.B: this has type string, but int is expected"), "{e}");
    let e = compile_err(r#"proc P(A): int { func A(): string; }"#);
    assert!(e.contains("ends proc P"), "{e}");
    let e = compile_err(r#"proc P(A): int { func A(): int; func B(i: int): int; A -> B(); }"#);
    assert!(e.contains("missing argument i"), "{e}");
    let e = compile_err(r#"proc P(A): int { func A(): int; func B(r: enum { X }): int; A -> B(r = Y(this)); }"#);
    assert!(e.contains("no variant Y"), "{e}");
}

#[test]
fn region_errors() {
    let e = compile_err(
        r#"proc P(A): int { func A(): []int; func B(i: int): int; A for each (A) -> B(i = this); }"#,
    );
    assert!(e.contains("without a collect"), "{e}");
    // D reads B, which only exists per element inside the region.
    let e = compile_err(
        r#"
        proc P(A): int {
            func A(): []int; func B(i: int): int; func C(l: []int): int; func D(i: int): int;
            A for each (A) -> B(i = this);
            B collect each (B) -> C(l = this);
            C -> D(i = B);
        }
        "#,
    );
    assert!(e.contains("not visible here"), "{e}");
}

#[test]
fn typedef_errors() {
    let e = compile_err("type A = B; type B = A; proc P(X): int { func X(): int; }");
    assert!(e.contains("type A refers to itself, so it must be a struct or enum"), "{e}");
    let e = compile_err("type L = []L; proc P(X): int { func X(): int; }");
    assert!(e.contains("type L refers to itself"), "{e}");
    let e = compile_err("type U = void; proc P(X): int { func X(): int; }");
    assert!(e.contains("type U names void"), "{e}");
    let e = compile_err("type int = string; proc P(X): int { func X(): int; }");
    assert!(e.contains("int is a built-in type"), "{e}");
    let e = compile_err("proc P(X): int { func X(t: T): int; }");
    assert!(e.contains("unknown type T"), "{e}");
}

#[test]
fn typedefs_are_structural() {
    // A typedef and its definition written out are interchangeable, both ways.
    compile(
        r#"
        type Pt = { x: int, y: int };
        proc P(A): int {
            func A(): { y: int, x: int };
            func B(p: Pt): Pt;
            func C(p: { x: int, y: int }): int;
            A -> B(p = this);
            B -> C(p = this);
        }
        "#,
    );
}

#[test]
fn literal_errors() {
    let base = |args: &str| format!(
        "type T = {{ a: int, b: enum {{ X, Y(string) }} }}; proc P(A): int {{ func A(): int; func B(t: T, f: float, l: []string): int; A -> B({args}); }}"
    );
    let e = compile_err(&base(r#"t = { a = 1 }, f = 1, l = []"#));
    assert!(e.contains("argument t of P.B: struct literal is missing field b of T"), "{e}");
    let e = compile_err(&base(r#"t = { a = 1, b = X(), c = 2 }, f = 1, l = []"#));
    assert!(e.contains("T has no field c"), "{e}");
    let e = compile_err(&base(r#"t = { a = "s", b = X() }, f = 1, l = []"#));
    assert!(e.contains("expected int, found a string literal"), "{e}");
    let e = compile_err(&base(r#"t = { a = 1, b = Y() }, f = 1, l = []"#));
    assert!(e.contains("variant Y carries string; write Y(value)"), "{e}");
    let e = compile_err(&base(r#"t = { a = 1, b = X() }, f = 1.5, l = [1]"#));
    assert!(e.contains("argument l of P.B: expected string, found an int literal"), "{e}");
    let e = compile_err(&base(r#"t = { a = 1, b = Z() }, f = 1, l = []"#));
    assert!(e.contains("has no variant Z"), "{e}");
    // An int literal is a fine float; everything else matches.
    compile(&base(r#"t = { b = Y("y"), a = -2 }, f = 1, l = ["a", "b"]"#));
}

#[test]
fn aliases_are_separate_nodes() {
    let src = r#"
        proc P(A): int {
            func A(): int;
            func B(i: int): int;
            func C(i: int): int;
            A switch { (this > 0) -> B(i = this) as B2, else -> B(i = 0) as B2 };
            B2 -> B(i = B2);
            B -> C(i = this) as C2;
        }
    "#;
    let prog = compile(src);
    let p = prog.proc(prog.top_procs[0]);
    let names: Vec<String> = p.graph.regions[0].nodes.iter().map(|&n| prog.node(n).name().to_string()).collect();
    assert_eq!(names, vec!["A", "B2", "B", "C2"]);
    let doc = vandorf::document::generate(&prog);
    let sub: Vec<_> = doc.grids["P"].rows.iter().flat_map(|r| &r.nodes).filter_map(|n| n.sublabel.clone()).collect();
    assert_eq!(sub, vec!["instance of B", "instance of C"]);

    let e = compile_err("proc P(A): int { func A(): int; func B(i: int): int; A -> B(i = this) as A; }");
    assert!(e.contains("alias A clashes with a node of that name in proc P"), "{e}");
    let e = compile_err(r#"proc P(A): int { func A(): int; func B(i: int): int; func C(i: int): int;
        A switch { (this > 0) -> B(i = this) as D, else -> C(i = this) as D }; }"#);
    assert!(e.contains("D is already an alias of B"), "{e}");
    let e = compile_err("proc P(A): int { func A(): int; func B(i: int): int; A -> B(i = this) as B2; B2 -> B2(i = this) as B3; }");
    assert!(e.contains("B2 is itself an alias"), "{e}");
}

#[test]
fn switch_errors() {
    let sw = |arms: &str| format!(
        "type E = enum {{ A(int), B, C({{ x: int, e: E }}) }}; proc P(S): int {{ func S(): E; func T(i: int): int; func Q(i: int): bool; S switch {{ {arms} }}; }}"
    );
    let e = compile_err(&sw("A(n) -> T(i = n), B() -> T(i = 0)"));
    assert!(e.contains("switch on S doesn't cover C(_); add an arm for it, or an else"), "{e}");
    let e = compile_err(&sw("A(_) -> T(i = 0), B() -> T(i = 0), C({ x = _, e = A(_) }) -> T(i = 1), C({ x = _, e = B() }) -> T(i = 2)"));
    assert!(e.contains("doesn't cover C({ x = _, e = C(_) })"), "{e}");
    let e = compile_err(&sw("A(_) -> T(i = 0), C({ x = _, e = _ }) | B() -> T(i = 1), B() -> T(i = 2)"));
    assert!(e.contains("this arm is never taken"), "{e}");
    let e = compile_err(&sw("A(_) -> T(i = 0), B() -> T(i = 1), C(_) -> T(i = 2), else -> T(i = 3)"));
    assert!(e.contains("else is never taken"), "{e}");
    let e = compile_err(&sw("else -> T(i = 0), B() -> T(i = 1)"));
    assert!(e.contains("comes after else"), "{e}");
    let e = compile_err(&sw("A(n) -> T(i = n), (true) -> T(i = 0), else -> T(i = 1)"));
    assert!(e.contains("this switch matches patterns (like A(n)), so its arms can't be conditions"), "{e}");
    let e = compile_err(&sw("A(n) -> switch { B() -> T(i = n), else -> T(i = 0) }, else -> T(i = 1)"));
    assert!(e.contains("a nested switch has nothing new to match"), "{e}");
    let e = compile_err(&sw("A(n) -> switch { (n > 0) -> T(i = n) }, else -> T(i = 1)"));
    assert!(e.contains("needs an else arm"), "{e}");
    let e = compile_err(&sw("B -> T(i = 0), else -> T(i = 1)"));
    assert!(e.contains("B is a variant of E; write B() to match it"), "{e}");
    let e = compile_err(&sw("A -> T(i = 0), else -> T(i = 1)"));
    assert!(e.contains("A is a variant of E; write A(_) to match it"), "{e}");
    let e = compile_err(&sw("A() -> T(i = 0), else -> T(i = 1)"));
    assert!(e.contains("A carries int; write A(_) to ignore it"), "{e}");
    let e = compile_err(&sw("B(_) -> T(i = 0), else -> T(i = 1)"));
    assert!(e.contains("B carries nothing; write B()"), "{e}");
    let e = compile_err(&sw("C({ x = n }) -> T(i = n), else -> T(i = 1)"));
    assert!(e.contains("struct pattern is missing field e of { x: int, e: E }; write e = _ to ignore it"), "{e}");
    let e = compile_err(&sw("C({ x = n, e = A(n) }) -> T(i = n), else -> T(i = 1)"));
    assert!(e.contains("n is bound twice"), "{e}");
    let e = compile_err(&sw("A(n) | B() -> T(i = 0), else -> T(i = 1)"));
    assert!(e.contains("or-patterns can't bind names yet"), "{e}");
    let e = compile_err(&sw("A(T) -> T(i = 0), else -> T(i = 1)"));
    assert!(e.contains("binding T hides a node"), "{e}");
    let e = compile_err(&sw("D() -> T(i = 0), else -> T(i = 1)"));
    assert!(e.contains("E has no variant D"), "{e}");
    let e = compile_err(&sw(r#"("this < 1") -> T(i = 0), else -> T(i = 1)"#));
    assert!(e.contains("write (this < 1), without quotes"), "{e}");
    let e = compile_err("proc P(S): int { func S(): int; func T(i: int): int; S switch { (this > 0) -> T(i = this) }; }");
    assert!(e.contains("this switch on S needs an else arm"), "{e}");
    let e = compile_err("proc P(S): int { func S(): int; func T(i: int): int; S switch { A(n) -> T(i = n), else -> T(i = 0) }; }");
    assert!(e.contains("which isn't an enum, so A(..) can't match it"), "{e}");
    // Exhaustive without an else: nested patterns, or-patterns, a nested
    // switch using a binding, and a binding catching the rest.
    compile(&sw(
        "A(n) -> switch { (Q(i = n) && n % 2 == 1) -> T(i = n * 2), else -> T(i = -n) }, \
         C({ x = x, e = A(_) | B() }) -> T(i = x), B() -> T(i = 0), other -> T(i = 9)",
    ));
    // A pattern switch on a struct, and binding the whole value.
    compile(
        "proc P(S): int { func S(): { a: bool, b: enum { X, Y(int) } }; func T(i: int): int; \
         S switch { { a = _, b = Y(n) } -> T(i = n), { a = _, b = X() } -> T(i = 0) }; }",
    );
}

#[test]
fn expression_errors() {
    let cond = |c: &str| format!(
        "proc P(S): int {{ func S(): {{ i: int, f: float, s: string, b: bool }}; func T(i: int): int; func Q(i: int): bool; func R(i: int); S switch {{ ({c}) -> T(i = 0), else -> T(i = 1) }}; }}"
    );
    let e = compile_err(&cond("this.i + this.f > 0"));
    assert!(e.contains("+ needs two operands of the same type (int, float, string), found int and float"), "{e}");
    let e = compile_err(&cond("this.s % 2 == 0"));
    assert!(e.contains("% needs two operands of the same type (int)"), "{e}");
    let e = compile_err(&cond("this.i && this.b"));
    assert!(e.contains("this.i has type int, but bool is expected"), "{e}");
    let e = compile_err(&cond("Q(this.i)"));
    assert!(e.contains("arguments of Q need names: Q(param = value)"), "{e}");
    let e = compile_err(&cond("R(i = 1) == 1"));
    assert!(e.contains("R returns nothing"), "{e}");
    let e = compile_err(&cond("P(x = 1)"));
    assert!(e.contains("P is not a func"), "{e}");
    let e = compile_err(&cond("Nope(i = 1)"));
    assert!(e.contains("Nope is neither a func nor a variant"), "{e}");
    // Int literals mix with floats; strings compare and concatenate.
    compile(&cond(r#"this.f * 2 > 1 && this.s + "x" != "y" && -this.i < 3 && !this.b"#));
}

#[test]
fn map_errors() {
    let m = |arg: &str| format!(
        "proc P(S): int {{ func S(): {{ m: map[string]int, xs: []int, s: string }}; func T(i: int): int; \
         func U(m: map[string]int): int; func K(ks: []string): int; S -> {arg}; }}"
    );
    let e = compile_err("proc P(S): int { func S(): map[float]int; }");
    assert!(e.contains("map keys are ints or strings, not float"), "{e}");
    let e = compile_err(&m("T(i = this.m[1])"));
    assert!(e.contains("argument i of P.T: expected string, found an int literal"), "{e}");
    let e = compile_err(&m("T(i = this.s[0])"));
    assert!(e.contains("only maps and arrays can be indexed, not string"), "{e}");
    let e = compile_err(&m(r#"U(m = this.m + ["a": "b"])"#));
    assert!(e.contains("expected int, found a string literal"), "{e}");
    let e = compile_err(&m(r#"U(m = ["a": 1, "a": 2])"#));
    assert!(e.contains(r#"key "a" given twice"#), "{e}");
    let e = compile_err(&m("U(m = this.m - this.xs)"));
    assert!(e.contains("this.xs has type []int, but []string is expected"), "{e}");
    let e = compile_err(&m("T(i = this.m.size)"));
    assert!(e.contains("map[string]int has no field size"), "{e}");
    let e = compile_err(&m("K(ks = this.xs.keys)"));
    assert!(e.contains("[]int has no field keys"), "{e}");
    let e = compile_err(&m("T(i = [:].len)"));
    assert!(e.contains("the type of this literal isn't known here"), "{e}");
    let e = compile_err("proc P(S): int { func S(): { m: map[string]int }; func T(b: bool): int; S -> T(b = 1 in this.m); }");
    assert!(e.contains("expected string, found an int literal"), "{e}");
    // Lookups, queries, merges and removals, and for each over keys.
    compile(&m(r#"U(m = this.m + ["n": this.m["x"] + this.xs[0] + this.s.len + this.m.values.len] - this.m.keys - ["y"])"#));
    compile(
        "proc P(S): int { func S(): map[string]{ x: int }; func T(k: string, x: int): int; func C(l: []int): int; \
         S for each (this.keys) -> T(k = this, x = S[this].x); T collect each (T) -> C(l = this); }",
    );
}

fn src(path: &str, text: &str) -> vandorf::Source {
    vandorf::Source { path: path.into(), text: text.into() }
}

#[test]
fn files_share_one_namespace() {
    // Types, interfaces and procs in one file are usable from another, in any order.
    let files = [
        src("a.sys", "proc A(S): int { func S(t: T): int; S -> B(i = this); }"),
        src("b.sys", "type T = { x: int };\nproc B(Q): int { func Q(i: int): int; }"),
    ];
    let prog = vandorf::check_files(&files).unwrap_or_else(|d| panic!("{}", vandorf::render(&files, &d, false)));
    assert_eq!(prog.top_procs.len(), 2);

    let files = [src("a.sys", "proc A(S): int { func S(): int; }"), src("b.sys", "\n\nproc A(S): int { func S(): int; }")];
    let d = vandorf::check_files(&files).unwrap_err();
    assert_eq!((d[0].file, d[0].msg.as_str()), (1, "duplicate name A"));
    let shown = vandorf::render(&files, &d, false);
    assert!(shown.contains("b.sys:3:6"), "{shown}");

    // Parse errors name their file too.
    let files = [src("ok.sys", "proc A(S): int { func S(): int; }"), src("bad.sys", "proc {")];
    let d = vandorf::check_files(&files).unwrap_err();
    assert_eq!(d[0].file, 1);
}

#[test]
fn projects_load_and_rebase_impl_paths() {
    let dir = std::env::temp_dir().join(format!("vandorf-project-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sys/sub")).unwrap();
    std::fs::create_dir_all(dir.join(".hidden")).unwrap();
    std::fs::write(
        dir.join("vandorf.toml"),
        "[project]\nname = \"t\"\nsources = [\"sys\"]\n[go]\nout = \"go/app\"\n[bin.tool]\nentry = \"A\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("sys/a.sys"), r#"proc A(S): int { func S(): int = "impl.go:S"; S -> B(); }"#).unwrap();
    std::fs::write(dir.join("sys/sub/b.sys"), r#"proc B(Q): int { func Q(): int = "../../go/q.go:Q"; }"#).unwrap();
    std::fs::write(dir.join(".hidden/c.sys"), "not even vandorf").unwrap();

    let found = vandorf::project::find(&dir.join("sys/sub")).unwrap();
    let p = vandorf::project::load(&found).unwrap();
    let paths: Vec<&str> = p.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["sys/a.sys", "sys/sub/b.sys"]);
    assert_eq!(p.manifest.bin["tool"].entry, "A");

    let mut prog = vandorf::check_files(&p.files).unwrap();
    vandorf::project::rebase_impls(&mut prog, &p.dirs(), &p.root.join("go/app"));
    let mut files: Vec<String> = prog
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            vandorf::sema::NodeKind::Func(f) => f.imps.first().map(|i| i.file.clone()),
            _ => None,
        })
        .collect();
    files.sort();
    assert_eq!(files, vec!["../../sys/impl.go", "../q.go"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn parse_errors_are_rendered() {
    let e = compile_err("proc P(A): int { func A(: int; }");
    assert!(e.contains("Error"), "{e}");
}

mod codegen {
    use vandorf::codegen::{MainSpec, go, rust};

    const SRC: &str = r#"
        proc P(A): int {
            func A(): { n: int } = "a.go:A", "../impls/a.rs:a";
            func B(i: int): int = "../other/b.go:B";
            A -> B(i = A.n);
        }
    "#;

    #[test]
    fn go_types_are_a_separate_package() {
        let prog = vandorf::compile("t.sys", SRC).unwrap();
        let out = go::generate(&prog, &go::GoOptions::new("app", Some("example.com/m/app".into()))).unwrap();
        assert_eq!(out.types.dir, "types");
        let p = &out.code.files["p_vandorf.go"];
        assert!(p.contains("\"example.com/m/app/types\""), "{p}");
        assert!(p.contains("other \"example.com/m/other\""), "{p}");
        assert!(p.contains("A(ctx)") && p.contains("other.B(ctx"), "{p}");
        assert!(out.types.files["types_vandorf.go"].starts_with("// Code generated"));
        // No module path: impls elsewhere and the types package can't be imported.
        assert!(go::generate(&prog, &go::GoOptions::new("app", None)).is_err());
    }

    #[test]
    fn go_types_in_code_package() {
        let prog = vandorf::compile("t.sys", r#"proc P(A): int { func A(): int; }"#).unwrap();
        let mut opts = go::GoOptions::new("app", None);
        opts.types_dir = ".".into();
        let out = go::generate(&prog, &opts).unwrap();
        assert_eq!(out.types.dir, "");
        assert!(out.code.files.contains_key("stubs_vandorf.go"));
    }

    #[test]
    fn rust_impl_paths_become_module_paths() {
        let prog = vandorf::compile("t.sys", SRC).unwrap();
        let out = rust::generate(&prog, &rust::RustOptions { module_dir: Some("out/app".into()), ..Default::default() }).unwrap();
        let procs = &out.code.files["procs.rs"];
        assert!(procs.contains("crate::out::impls::a::a()"), "{procs}");
        // B has no .rs reference: stubbed.
        assert!(procs.contains("fn unimplemented_p_b("), "{procs}");
        assert!(out.impls[0].signature.contains("crate::out::app::types::PAOut"), "{:?}", out.impls);
        assert!(rust::generate(&prog, &rust::RustOptions::default()).is_err());
    }

    const MAIN: &str = r#"
        proc Main(A): string {
            func A(n: int, verbose: bool, name: string): string;
        }
    "#;

    fn spec(name: &str, entry: &str, path: &str) -> MainSpec {
        MainSpec { name: name.into(), entry: entry.into(), path: path.into() }
    }

    #[test]
    fn go_main_runs_entry_proc() {
        let prog = vandorf::compile("t.sys", MAIN).unwrap();
        let mut opts = go::GoOptions::new("app", Some("example.com/m/app".into()));
        opts.mains = vec![spec("main", "Main", "main"), spec("tool", "Main", "../cmd/tool")];
        let mains = go::generate(&prog, &opts).unwrap().mains;
        assert_eq!(mains.iter().map(|m| m.dir.as_str()).collect::<Vec<_>>(), vec!["main", "../cmd/tool"]);
        let src = &mains[0].files["main_vandorf.go"];
        assert!(src.contains("package main") && src.contains("app \"example.com/m/app\""), "{src}");
        assert!(src.contains("strconv.Atoi(os.Args[1])") && src.contains("strconv.ParseBool(os.Args[2])"), "{src}");
        assert!(src.contains("app.RunMain(context.Background(), argN, argVerbose, argName)"), "{src}");
        assert!(mains[1].files["main_vandorf.go"].contains("usage: tool <n: int>"));
        // None by default.
        assert!(go::generate(&prog, &go::GoOptions::new("app", Some("x".into()))).unwrap().mains.is_empty());
    }

    #[test]
    fn rust_main_runs_entry_proc() {
        let prog = vandorf::compile("t.sys", "proc Tool(A): string { func A(n: int): string; }").unwrap();
        let opts = rust::RustOptions {
            module_dir: Some("out/app".into()),
            mains: vec![spec("main", "Tool", "main.rs"), spec("tool", "Tool", "bin/tool.rs")],
            crate_name: Some("demo".into()),
        };
        let mains = rust::generate(&prog, &opts).unwrap().mains;
        assert_eq!(mains[0].dir, "../..");
        assert_eq!((mains[1].dir.as_str(), mains[1].files.keys().next().unwrap().as_str()), ("../../bin", "tool.rs"));
        let src = &mains[0].files["main.rs"];
        assert!(src.contains("demo::out::app::run_tool(arg_n)"), "{src}");
        // Needs the crate name.
        let opts = rust::RustOptions { crate_name: None, ..opts };
        assert!(rust::generate(&prog, &opts).unwrap_err().contains("lib.rs"));
    }

    #[test]
    fn main_requires_entry_proc_with_primitive_params() {
        let mut opts = go::GoOptions::new("app", Some("x".into()));
        opts.mains = vec![spec("calc", "Calc", "main")];
        let no_main = vandorf::compile("t.sys", SRC).unwrap();
        assert!(go::generate(&no_main, &opts).unwrap_err().contains("calc runs proc Calc, but there is no top-level proc Calc"));
        let arr = vandorf::compile("t.sys", "proc Calc(A): int { func A(l: []int): int; }").unwrap();
        assert!(go::generate(&arr, &opts).unwrap_err().contains("parameter l"));
    }
}
