//! Code generators. Each backend works from a checked [`crate::sema::Program`]
//! and produces two artifacts:
//!
//! - **types** ("typegen"): a named type for every struct and enum in the
//!   program, from the shared [`types::TypeTable`]. It depends on nothing, so
//!   implementations can import it.
//! - **code** ("codegen"): one state machine per proc, calling the
//!   implementations. It depends on the types artifact and the implementations.
//!
//! On request, a third artifact, **main**, is an executable entry point that
//! runs the top-level proc `Main` with its parameters from the command line.

pub mod go;
pub mod rust;
pub mod types;

use crate::sema::{NodeId, NodeKind, Program};
use crate::types::{Prim, Type};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct Output {
    pub types: Artifact,
    pub code: Artifact,
    /// The executable entry point, when requested.
    pub main: Option<Artifact>,
    /// Every implementation the code calls, for the caller to verify.
    pub impls: Vec<ImplUse>,
}

#[derive(Debug, Clone)]
pub struct Artifact {
    /// Directory relative to the output directory; empty for the output
    /// directory itself.
    pub dir: String,
    /// File name -> source.
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ImplUse {
    pub node: String,
    pub file: String,
    pub function: String,
    /// The signature the implementation must have.
    pub signature: String,
}

/// Name of the proc the main artifact runs.
pub const MAIN_PROC: &str = "Main";

/// Whether the program has a top-level proc `Main`.
pub fn has_main(prog: &Program) -> bool {
    main_proc(prog).is_ok()
}

/// The top-level proc `Main` and its parameters, which must be primitives
/// (they are parsed from command-line arguments).
pub(crate) fn main_proc(prog: &Program) -> Result<(NodeId, Vec<(String, Prim)>), String> {
    let pid = prog
        .top_procs
        .iter()
        .copied()
        .find(|&p| prog.node(p).name() == MAIN_PROC && matches!(prog.node(p).kind, NodeKind::Proc(_)))
        .ok_or_else(|| format!("generating a main needs a top-level proc {MAIN_PROC}, and there is none"))?;
    let params = prog
        .params(pid)
        .iter()
        .map(|(name, t)| match t {
            Type::Prim(p) if *p != Prim::Void => Ok((name.clone(), *p)),
            t => Err(format!(
                "{MAIN_PROC}'s parameter {name} has type {t}; main reads its parameters from the command line, so they must be int, float, bool or string"
            )),
        })
        .collect::<Result<_, _>>()?;
    Ok((pid, params))
}

/// `usage: <bin> <a: int> <b: string>`
pub(crate) fn main_usage(bin: &str, params: &[(String, Prim)]) -> String {
    let mut s = format!("usage: {bin}");
    for (n, p) in params {
        s.push_str(&format!(" <{n}: {}>", Type::Prim(*p)));
    }
    s
}

/// Name of a node in generated code: its qualified name, segments
/// capitalized and joined (`Bar.Start` -> `BarStart`).
pub(crate) fn gname(prog: &Program, n: NodeId) -> String {
    prog.node(n).qual.iter().map(|s| exported(s)).collect()
}

pub(crate) fn exported(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
        None => String::new(),
    }
}

/// `BarStart` -> `bar_start`
pub(crate) fn snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A double-quoted string literal, valid in both Go and Rust.
pub(crate) fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Joins `rel` onto a `/`-separated `base`, resolving `.` and `..`
/// lexically. `None` if `..` climbs above the root of `base`.
pub(crate) fn join_path(base: &str, rel: &str) -> Option<String> {
    let mut segs: Vec<&str> = base.split('/').filter(|s| !s.is_empty() && *s != ".").collect();
    for s in rel.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                segs.pop()?;
            }
            s => segs.push(s),
        }
    }
    Some(segs.join("/"))
}

/// Directory part of a `/`-separated path (`""` if none).
pub(crate) fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Emits lines with indentation inferred from brackets, so output is
/// readable without a formatter: a line that starts by closing a bracket
/// dedents, a line that ends by opening one indents. Go `case`/`default:`
/// lines hang one level out.
pub(crate) struct Writer {
    pub out: String,
    depth: usize,
    indent: &'static str,
}

impl Writer {
    pub fn new(indent: &'static str) -> Writer {
        Writer { out: String::new(), depth: 0, indent }
    }

    pub fn line(&mut self, s: &str) {
        let s = s.trim();
        if s.starts_with(['}', ')', ']']) {
            self.depth = self.depth.saturating_sub(1);
        }
        let hang = s.starts_with("case ") || s == "default:";
        let depth = if hang { self.depth.saturating_sub(1) } else { self.depth };
        if !s.is_empty() {
            self.out.push_str(&self.indent.repeat(depth));
        }
        self.out.push_str(s);
        self.out.push('\n');
        if s.ends_with(['{', '(', '[']) {
            self.depth += 1;
        }
    }

    pub fn raw(&mut self, s: &str) {
        self.out.push_str(s);
    }
}
