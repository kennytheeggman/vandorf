//! vcc's diagnostic snippets (message, path:line:col, the line underlined,
//! notes), against their golden files.

mod common;

#[test]
fn renders_snippets() {
    let cases: &[(&str, std::ops::Range<usize>, &[&str])] = &[
        ("proc P(): int { func A(: int; }", 23..24, &[]),
        ("a\n\tfunc B(): int;\n", 8..9, &["expected: pub fn b()"]),
        ("a\n\tfunc B(): int;\n", 3..17, &["one", "two"]),
        ("type T = { a: int\n", 18..18, &[]),
        ("", 0..0, &[]),
        ("é", 1..9, &[]),
        ("x\r\ny\r\n", 3..4, &[]),
        ("multi\nline\nspan\n", 2..14, &[]),
        (&"line\n".repeat(120), 500..504, &[]),
    ];
    for (i, (text, span, notes)) in cases.iter().enumerate() {
        let notes: Vec<String> = notes.iter().map(|n| n.to_string()).collect();
        let mine = vcc::vcc::run_render_snippet(vcc::vcc::types::RenderSnippetS {
            level: "error".into(),
            msg: "msg".into(),
            path: "f.sys".into(),
            text: (*text).into(),
            at: span.start as i64,
            end: span.end as i64,
            notes: notes.iter().map(|n| n.as_str().into()).collect(),
        })
        .unwrap();
        common::check("report", &format!("c{i}"), &mine);
    }
}
