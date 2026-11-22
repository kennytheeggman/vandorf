//! Code generators. Each backend works from a checked [`crate::sema::Program`]
//! and produces two artifacts:
//!
//! - **types** ("typegen"): a named type for every struct and enum in the
//!   program, from the shared [`types::TypeTable`]. It depends on nothing, so
//!   implementations can import it.
//! - **code** ("codegen"): one state machine per proc, calling the
//!   implementations. It depends on the types artifact and the implementations.
//!
//! On request, more artifacts, **mains**: executables that each run a
//! top-level proc (the entry, `Main` by default) with its parameters from the
//! command line. See [`MainSpec`].

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
    /// One per requested [`MainSpec`].
    pub mains: Vec<Artifact>,
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
    /// The func's node, and its qualified name.
    pub id: crate::sema::NodeId,
    pub node: String,
    pub file: String,
    pub function: String,
    /// The signature the implementation must have.
    pub signature: String,
}

/// The default entry proc.
pub const MAIN_PROC: &str = "Main";

/// An executable to generate.
#[derive(Debug, Clone)]
pub struct MainSpec {
    /// The executable's name, for usage messages.
    pub name: String,
    /// The top-level proc it runs.
    pub entry: String,
    /// Where it goes. Go: the main package's directory, relative to the
    /// output directory. Rust: the file, relative to the crate's `src/`
    /// (`main.rs`, `bin/<name>.rs`).
    pub path: String,
}

/// Whether the program has a top-level proc named `name`.
pub fn has_proc(prog: &Program, name: &str) -> bool {
    find_proc(prog, name).is_some()
}

pub(crate) fn find_proc(prog: &Program, name: &str) -> Option<NodeId> {
    prog.top_procs.iter().copied().find(|&p| prog.node(p).name() == name && matches!(prog.node(p).kind, NodeKind::Proc(_)))
}

/// How a main passes the command line to its entry proc.
pub(crate) enum MainParams {
    /// Each parameter a primitive, parsed from one argument, in order.
    Typed(Vec<(String, Prim)>),
    /// A single `[]string` parameter: the arguments as they are.
    Argv(String),
}

/// A main's entry proc and its parameters: primitives (parsed from the
/// command-line arguments), or one `[]string` that takes them all.
pub(crate) fn main_proc(prog: &Program, m: &MainSpec) -> Result<(NodeId, MainParams), String> {
    let entry = &m.entry;
    let pid = find_proc(prog, entry)
        .ok_or_else(|| format!("{} runs proc {entry}, but there is no top-level proc {entry}", m.name))?;
    if let [(name, t)] = &prog.params(pid)[..]
        && let Type::Array(e) = t.unfold(&prog.types)
        && matches!(e.unfold(&prog.types), Type::Prim(Prim::String))
    {
        return Ok((pid, MainParams::Argv(name.clone())));
    }
    let params = prog
        .params(pid)
        .iter()
        .map(|(name, t)| match t.unfold(&prog.types) {
            Type::Prim(p) if *p != Prim::Void => Ok((name.clone(), *p)),
            t => Err(format!(
                "{entry}'s parameter {name} has type {t}; {} reads its parameters from the command line, so they must be int, float, bool or string (or a single []string, for all the arguments)",
                m.name,
            )),
        })
        .collect::<Result<_, _>>()?;
    Ok((pid, MainParams::Typed(params)))
}

/// `usage: <bin> <a: int> <b: string>`, or `usage: <bin> <args...>`
pub(crate) fn main_usage(bin: &str, params: &MainParams) -> String {
    let mut s = format!("usage: {bin}");
    match params {
        MainParams::Typed(ps) => {
            for (n, p) in ps {
                s.push_str(&format!(" <{n}: {}>", Type::Prim(*p)));
            }
        }
        MainParams::Argv(n) => s.push_str(&format!(" <{n}...>")),
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
