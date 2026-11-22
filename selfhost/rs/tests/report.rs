//! vcc's diagnostics render as the reference's vandorf::report::snippet.

#[test]
fn snippets_match_the_reference() {
    let cases: &[(&str, std::ops::Range<usize>, &[&str])] = &[
        ("proc P(A): int { func A(: int; }", 24..25, &[]),
        ("a\n\tfunc B(): int;\n", 8..9, &["expected: pub fn b()"]),
        ("a\n\tfunc B(): int;\n", 3..17, &["one", "two"]),
        ("type T = { a: int\n", 18..18, &[]),
        ("", 0..0, &[]),
        ("é", 1..9, &[]),
        ("x\r\ny\r\n", 3..4, &[]),
        ("multi\nline\nspan\n", 2..14, &[]),
        (&"line\n".repeat(120), 500..504, &[]),
    ];
    for (text, span, notes) in cases {
        let notes: Vec<String> = notes.iter().map(|n| n.to_string()).collect();
        let want = vandorf::report::snippet("error", "msg", "f.sys", text, span.clone(), &notes);
        let mine = vcc::vcc::run_render_snippet(vcc::vcc::types::RenderSnippetAtS {
            level: "error".into(),
            msg: "msg".into(),
            path: "f.sys".into(),
            text: (*text).into(),
            at: span.start as i64,
            end: span.end as i64,
            notes: notes.iter().map(|n| n.as_str().into()).collect(),
        })
        .unwrap();
        assert_eq!(&*mine, want, "{text:?} {span:?}");
    }
}
