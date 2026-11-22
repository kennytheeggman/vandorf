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
pub mod report;

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
                msg: parse_message(e),
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

/// A parse error as `expected a, b or c, found x`.
fn parse_message(e: &parser::ParseError) -> String {
    use chumsky::error::{RichPattern, RichReason};
    match e.reason() {
        RichReason::ExpectedFound { expected, found } => {
            let mut want: Vec<String> = vec![];
            for p in expected {
                let s = match p {
                    // A comment could start anywhere whitespace can: not news.
                    RichPattern::Token(t) if **t == '/' => continue,
                    RichPattern::Token(t) => format!("{:?}", &**t),
                    RichPattern::Label(l) if l.starts_with(['a', 'e', 'i', 'o', 'u']) => format!("an {l}"),
                    RichPattern::Label(l) => format!("a {l}"),
                    RichPattern::Identifier(k) => k.clone(),
                    RichPattern::EndOfInput => "the end of the file".into(),
                    _ => continue,
                };
                if !want.contains(&s) {
                    want.push(s);
                }
            }
            let found = found.as_ref().map_or("the end of the file".to_string(), |t| format!("{:?}", &**t));
            match want.as_slice() {
                [] => format!("unexpected {found}"),
                [one] => format!("expected {one}, found {found}"),
                [init @ .., last] => format!("expected {} or {last}, found {found}", init.join(", ")),
            }
        }
        RichReason::Custom(m) => m.to_string(),
    }
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
    check_files(files).map_err(|d| render(files, &d))
}

/// Diagnostics as [`report::snippet`]s, a blank line apart (each once).
pub fn render(files: &[Source], diags: &[Diagnostic]) -> String {
    let mut seen = std::collections::HashSet::new();
    diags
        .iter()
        .filter(|d| seen.insert((d.file, d.span.clone(), d.msg.clone())))
        .map(|d| {
            let f = &files[d.file];
            report::snippet("error", &d.msg, &f.path, &f.text, d.span.clone(), &[])
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
