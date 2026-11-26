//! Implementation of the Git interface (git.sys): runs the git command line.

use crate::vcc::types::*;
use std::path::Path;
use std::process::{Command, Stdio};

fn err_str(msg: String) -> ResString {
    ResString::Err(Diag {
        msg: msg.into(),
        span: Span {
            at: 0,
            end: 0,
            file: 0,
        },
    })
}

/// Runs git in dir with args (and env), giving its output, or its error
/// output (else its output, else its status) as the error.
fn git(dir: &str, args: &[&str], env: &[(&str, &Path)]) -> Result<Vec<u8>, String> {
    let mut c = Command::new("git");
    c.args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let err = if err.is_empty() {
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    } else {
        err
    };
    Err(if err.is_empty() {
        format!("git {} failed: {}", args.join(" "), out.status)
    } else {
        err.strip_prefix("fatal: ").unwrap_or(&err).to_string()
    })
}

fn text(out: Vec<u8>) -> String {
    String::from_utf8_lossy(&out).into_owned()
}

/// Runs git in dir with args, giving its output without trailing
/// whitespace, or its error output as the error.
pub fn run(dir: Str, args: List<Str>) -> Result<ResString, String> {
    let args: Vec<&str> = args.iter().map(|a| &**a).collect();
    Ok(match git(&dir, &args, &[]) {
        Ok(out) => ResString::Ok(text(out).trim_end().into()),
        Err(e) => err_str(e),
    })
}

/// Reads the file at path, relative to the top of dir's worktree, as it is
/// in commit rev (git show rev:path), byte for byte.
pub fn show(dir: Str, rev: Str, path: Str) -> Result<ResString, String> {
    // cat-file is show without the conversions a config can add.
    let object = format!("{rev}:{path}");
    Ok(match git(&dir, &["cat-file", "blob", &object], &[]) {
        Ok(out) => ResString::Ok(text(out).into()),
        Err(e) => err_str(e),
    })
}

/// Writes the files of dir's working tree, tracked and untracked but not
/// ignored, as a tree, through a temporary copy of the index, so the
/// worktree's own index is left as it is; gives the tree's id.
pub fn snapshot(dir: Str) -> Result<ResString, String> {
    let snap = || -> Result<String, String> {
        let gitdir = text(git(&dir, &["rev-parse", "--absolute-git-dir"], &[])?);
        let gitdir = Path::new(gitdir.trim_end());
        let tmp = gitdir.join(format!("vcc-snapshot-{}.index", std::process::id()));
        // Starting from a copy of the index, only what changed is hashed.
        let index = gitdir.join("index");
        if index.is_file() {
            std::fs::copy(&index, &tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        }
        let env = [("GIT_INDEX_FILE", tmp.as_path())];
        let tree = git(&dir, &["add", "--all"], &env)
            .and_then(|_| git(&dir, &["write-tree"], &env))
            .map(|t| text(t).trim().to_string());
        let _ = std::fs::remove_file(&tmp);
        tree
    };
    Ok(match snap() {
        Ok(tree) => ResString::Ok(tree.into()),
        Err(e) => err_str(e),
    })
}
