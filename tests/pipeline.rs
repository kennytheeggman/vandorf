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
