//! vandorf: a language of procs whose only control flow is tail calls,
//! `switch`, `for each` and `collect`.
//!
//! Pipeline: [`parser`] (source -> [`ast`]) -> [`sema`] (names, types,
//! regions) -> [`document`] (layout for the visualizer) and [`codegen`].

pub mod ast;
pub mod codegen;
pub mod document;
mod exhaust;
pub mod parser;
pub mod sema;
pub mod types;

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind, Source};
use std::ops::Range;

/// An error at a byte range of the source.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub msg: String,
    pub span: Range<usize>,
}

/// Parses and checks `src`.
pub fn check(src: &str) -> Result<sema::Program, Vec<Diagnostic>> {
    let file = parser::parse(src).map_err(|errs| {
        errs.iter().map(|e| Diagnostic { msg: e.to_string(), span: e.span().into_range() }).collect::<Vec<_>>()
    })?;
    sema::analyze(&file).map_err(|errs| {
        errs.into_iter().map(|e| Diagnostic { msg: e.msg, span: e.span.into_range() }).collect()
    })
}

/// Parses and checks `src`, rendering any diagnostics for a terminal.
pub fn compile(name: &str, src: &str) -> Result<sema::Program, String> {
    check(src).map_err(|d| render(name, src, &d, true))
}

pub fn render(name: &str, src: &str, diags: &[Diagnostic], color: bool) -> String {
    let config = Config::default().with_color(color).with_index_type(IndexType::Byte);
    let mut out = Vec::new();
    for d in diags {
        Report::build(ReportKind::Error, (name, d.span.clone()))
            .with_config(config)
            .with_message(&d.msg)
            .with_label(Label::new((name, d.span.clone())).with_message(&d.msg).with_color(Color::Red))
            .finish()
            .write((name, Source::from(src)), &mut out)
            .unwrap();
    }
    String::from_utf8_lossy(&out).into_owned()
}
