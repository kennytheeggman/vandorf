//! Diagnostics for people: the message, where (`path:line:col`), and the
//! line with the span underlined, rustc-style. Plain text, no color, so
//! vcc (selfhost/sys/cli/cli_leaf.rs:snippet) renders the same bytes.
//!
//! ```text
//! error: P.A ends proc P but returns string, expected int
//!  --> sys/main.sys:3:2
//!   |
//! 3 |     A -> id("x");
//!   |     ^^^^^^^^^^^^
//!   = expected: pub fn a(n: i64) -> Result<i64, E>
//! ```

use std::ops::Range;

/// One diagnostic: `level` ("error", "warning") and `msg`, at `span` (byte
/// offsets) of `text`, the file at `path`; `notes` follow as `= note` lines.
pub fn snippet(level: &str, msg: &str, path: &str, text: &str, span: Range<usize>, notes: &[String]) -> String {
    let at = floor(text, span.start.min(text.len()));
    let end = floor(text, span.end.clamp(at, text.len()));
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    let line_no = text[..at].matches('\n').count() + 1;
    let col = text[line_start..at].chars().count() + 1;
    let line = text[line_start..line_end].trim_end_matches('\r');
    // Tabs shown as four spaces; the underline follows the shown text.
    let width = |s: &str| s.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>();
    let shown = line.replace('\t', "    ");
    let lead = width(&line[..(at - line_start).min(line.len())]);
    let marked = width(&line[(at - line_start).min(line.len())..(end.min(line_end) - line_start).min(line.len())]).max(1);
    let pad = " ".repeat(line_no.to_string().len());
    let mut out = format!("{level}: {msg}\n{pad}--> {path}:{line_no}:{col}\n{pad} |\n{line_no} | {shown}\n{pad} | {}{}", " ".repeat(lead), "^".repeat(marked));
    for n in notes {
        out.push_str(&format!("\n{pad} = {n}"));
    }
    out
}

/// The nearest char boundary at or before `i`.
fn floor(text: &str, mut i: usize) -> usize {
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}
