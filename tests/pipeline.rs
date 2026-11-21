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
            C switch { ("this < 3") -> A(i = this), ("else") -> D(i = this) };
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
            A switch { ("L") -> B(), ("R") -> C() };
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
    assert!(e.contains("expects int"), "{e}");
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
fn parse_errors_are_rendered() {
    let e = compile_err("proc P(A): int { func A(: int; }");
    assert!(e.contains("Error"), "{e}");
}

mod codegen {
    use vandorf::codegen::{go, rust};

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

    #[test]
    fn go_main_runs_proc_main() {
        let prog = vandorf::compile("t.sys", MAIN).unwrap();
        let mut opts = go::GoOptions::new("app", Some("example.com/m/app".into()));
        opts.main = true;
        let main = go::generate(&prog, &opts).unwrap().main.unwrap();
        assert_eq!(main.dir, "main");
        let src = &main.files["main_vandorf.go"];
        assert!(src.contains("package main") && src.contains("app \"example.com/m/app\""), "{src}");
        assert!(src.contains("strconv.Atoi(os.Args[1])") && src.contains("strconv.ParseBool(os.Args[2])"), "{src}");
        assert!(src.contains("app.RunMain(context.Background(), argN, argVerbose, argName)"), "{src}");
        // Off by default.
        assert!(go::generate(&prog, &go::GoOptions::new("app", Some("x".into()))).unwrap().main.is_none());
    }

    #[test]
    fn rust_main_runs_proc_main() {
        let prog = vandorf::compile("t.sys", MAIN).unwrap();
        let opts = rust::RustOptions { module_dir: Some("out/app".into()), main: true, crate_name: Some("demo".into()) };
        let main = rust::generate(&prog, &opts).unwrap().main.unwrap();
        assert_eq!(main.dir, "../..");
        let src = &main.files["main.rs"];
        assert!(src.contains("demo::out::app::run_main(arg_n, arg_verbose, arg_name)"), "{src}");
        assert!(src.contains("args[1].parse::<bool>()"), "{src}");
        // Needs the crate name.
        let opts = rust::RustOptions { crate_name: None, ..opts };
        assert!(rust::generate(&prog, &opts).unwrap_err().contains("lib.rs"));
    }

    #[test]
    fn main_requires_proc_main_with_primitive_params() {
        let mut opts = go::GoOptions::new("app", Some("x".into()));
        opts.main = true;
        let no_main = vandorf::compile("t.sys", SRC).unwrap();
        assert!(go::generate(&no_main, &opts).unwrap_err().contains("proc Main"));
        let arr = vandorf::compile("t.sys", "proc Main(A): int { func A(l: []int): int; }").unwrap();
        assert!(go::generate(&arr, &opts).unwrap_err().contains("parameter l"));
    }
}
