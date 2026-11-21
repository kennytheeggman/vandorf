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

pub mod project;

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind};
use std::ops::Range;

/// A source file: its path (for messages and relative impl paths) and text.
#[derive(Debug, Clone)]
pub struct Source {
    pub path: String,
    pub text: String,
}

/// An error at a byte range of one of the sources.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub msg: String,
    /// Index into the sources.
    pub file: ast::FileId,
    pub span: Range<usize>,
}

/// Parses and checks several files as one program: they share one
/// namespace, so any file can name any other's declarations.
pub fn check_files(files: &[Source]) -> Result<sema::Program, Vec<Diagnostic>> {
    let mut all = ast::File::default();
    let mut diags = vec![];
    for (id, f) in files.iter().enumerate() {
        match parser::parse(&f.text, id) {
            Ok(file) => {
                all.types.extend(file.types);
                all.interfaces.extend(file.interfaces);
                all.nodes.extend(file.nodes);
            }
            Err(errs) => diags.extend(errs.iter().map(|e| Diagnostic {
                msg: e.to_string(),
                file: e.span().context,
                span: e.span().into_range(),
            })),
        }
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    sema::analyze(&all).map_err(|errs| {
        errs.into_iter().map(|e| Diagnostic { msg: e.msg, file: e.span.context, span: e.span.into_range() }).collect()
    })
}

/// Parses and checks one file.
pub fn check(src: &str) -> Result<sema::Program, Vec<Diagnostic>> {
    check_files(&[Source { path: "<input>".into(), text: src.into() }])
}

/// Parses and checks `src`, rendering any diagnostics for a terminal.
pub fn compile(name: &str, src: &str) -> Result<sema::Program, String> {
    compile_files(&[Source { path: name.into(), text: src.into() }])
}

/// Like [`check_files`], rendering any diagnostics for a terminal.
pub fn compile_files(files: &[Source]) -> Result<sema::Program, String> {
    check_files(files).map_err(|d| render(files, &d, true))
}

pub fn render(files: &[Source], diags: &[Diagnostic], color: bool) -> String {
    let config = Config::default().with_color(color).with_index_type(IndexType::Byte);
    let mut cache = ariadne::sources(files.iter().map(|f| (f.path.clone(), f.text.clone())));
    let mut out = Vec::new();
    for d in diags {
        let name = files[d.file].path.clone();
        Report::build(ReportKind::Error, (name.clone(), d.span.clone()))
            .with_config(config)
            .with_message(&d.msg)
            .with_label(Label::new((name, d.span.clone())).with_message(&d.msg).with_color(Color::Red))
            .finish()
            .write(&mut cache, &mut out)
            .unwrap();
    }
    String::from_utf8_lossy(&out).into_owned()
}
