//! vandorf: a language of procs whose only control flow is tail calls,
//! `switch`, `for each` and `collect`.
//!
//! Pipeline: [`parser`] (source -> [`ast`]) -> [`sema`] (names, types,
//! regions) -> [`document`] (layout for the visualizer) and [`codegen`].

pub mod ast;
pub mod codegen;
pub mod document;
pub mod parser;
pub mod sema;
pub mod types;

use ariadne::{Color, Label, Report, ReportKind, Source};

/// Parses and checks `src`, rendering any diagnostics into the error string.
pub fn compile(name: &str, src: &str) -> Result<sema::Program, String> {
    let file = parser::parse(src).map_err(|errs| {
        render(name, src, errs.iter().map(|e| (e.to_string(), e.span().into_range())))
    })?;
    sema::analyze(&file)
        .map_err(|errs| render(name, src, errs.iter().map(|e| (e.msg.clone(), e.span.into_range()))))
}

fn render(name: &str, src: &str, errs: impl Iterator<Item = (String, std::ops::Range<usize>)>) -> String {
    let mut out = Vec::new();
    for (msg, span) in errs {
        Report::build(ReportKind::Error, (name, span.clone()))
            .with_message(&msg)
            .with_label(Label::new((name, span)).with_message(&msg).with_color(Color::Red))
            .finish()
            .write((name, Source::from(src)), &mut out)
            .unwrap();
    }
    String::from_utf8_lossy(&out).into_owned()
}
