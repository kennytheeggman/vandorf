//! Leaves for git.sys: paths as git lists and takes them.

use crate::vcc::types::*;

/// Splits the output of git ls-tree -z into the paths of the .sys files
/// under the project's directory dir, relative to it, sorted; files in hidden
/// directories and build output (target, node_modules) are left out, as
/// Fs.Walk leaves them out.
pub fn sys_files(listing: Str, dir: Str) -> Result<List<Str>, String> {
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{}/", dir.trim_end_matches('/'))
    };
    let mut out: Vec<Str> = listing
        .split('\0')
        .filter_map(|p| p.strip_prefix(prefix.as_str()))
        .filter(|p| p.ends_with(".sys"))
        .filter(|p| {
            let segs: Vec<&str> = p.split('/').collect();
            segs[..segs.len() - 1]
                .iter()
                .all(|s| !s.starts_with('.') && *s != "target" && *s != "node_modules")
        })
        .map(Str::from)
        .collect();
    out.sort();
    Ok(out.into())
}

/// Joins a path under the top of a worktree as git takes it: "." for the top
/// itself.
pub fn git_path(dir: Str, path: Str) -> Result<Str, String> {
    let p = super::cli_leaf::join(dir, path)?;
    Ok(if p.is_empty() { ".".into() } else { p })
}

/// Names a worktree's review worktree after it: its directory's name and a
/// hash of its path, so each worktree has one of its own.
pub fn review_name(top: Str) -> Result<Str, String> {
    // FNV-1a: stable across runs and builds.
    let hash = top.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    let name: String = top
        .rsplit('/')
        .next()
        .unwrap_or("")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Ok(format!("vcc-review-{name}-{:08x}", hash as u32).into())
}

/// Shortens a commit id to its first seven characters, as git shows them.
pub fn short_id(commit: Str) -> Result<Str, String> {
    Ok(commit.chars().take(7).collect::<String>().into())
}
