//! `vcc diff`, `vcc base` and `vcc review` on git repositories made for the
//! test: the base, per worktree; sides read from commits and the working
//! tree, one that doesn't check, and a project below the top of the
//! repository; the review worktree's HEAD, index and files, and its removal;
//! and the live worktree left as it was.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim_end().to_string()
}

/// vcc's exit status, stdout and stderr, run in dir.
fn vcc(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_vcc")).args(args).arg("-C").arg(dir).stdin(Stdio::null()).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

const MANIFEST: &str = "[project]\nname = \"t\"\nsources = [\"sys\"]\n";

const P1: &str = "proc P(n: int): int {\n\tfunc A(n: int): int;\n\tfunc B(n: int): int;\n\tP -> A(n = this.n);\n\tA -> B(n = this);\n}\n";
const P2: &str = "proc P(n: int): int {\n\tfunc A(n: int): int;\n\tfunc B(n: int): int;\n\tfunc C(n: int): int;\n\tP -> A(n = this.n);\n\tA -> B(n = this);\n\tB -> C(n = this);\n}\n";
const Q1: &str = "proc Q(n: int): int {\n\tfunc A(n: int): int;\n\tQ -> A(n = this.n);\n}\n";
const Q2: &str = "proc Q(n: int): int {\n\tfunc A(n: int): int;\n\tQ -> A(n = this.n + 1);\n}\n";

/// A repository with the project in proj/ (a.sys holding P, b.sys Q): a
/// first commit, then a second changing P. Gives its top and both commits.
fn repo(tag: &str) -> (PathBuf, String, String) {
    let dir = std::env::temp_dir().join(format!("vcc-git-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj/sys")).unwrap();
    let dir = dunce::canonicalize(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join("proj/vandorf.toml"), MANIFEST).unwrap();
    std::fs::write(dir.join("proj/sys/a.sys"), P1).unwrap();
    std::fs::write(dir.join("proj/sys/b.sys"), Q1).unwrap();
    std::fs::write(dir.join("README"), "a repository\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-qm", "first"]);
    let c1 = git(&dir, &["rev-parse", "HEAD"]);
    std::fs::write(dir.join("proj/sys/a.sys"), P2).unwrap();
    git(&dir, &["commit", "-qam", "second"]);
    let c2 = git(&dir, &["rev-parse", "HEAD"]);
    (dir, c1, c2)
}

/// What git says about a worktree's HEAD, index and files.
fn state(dir: &Path) -> (String, String, String) {
    (git(dir, &["rev-parse", "HEAD"]), git(dir, &["diff", "--cached", "--name-status"]), git(dir, &["status", "--porcelain"]))
}

#[test]
fn the_base_is_per_worktree() {
    let (dir, c1, c2) = repo("base");
    // Unset, the base is HEAD, and a note says so.
    let (ok, out, err) = vcc(&dir, &["base"]);
    assert!(ok && out.trim() == c2 && err.contains("refs/worktree/base is unset"), "{out}{err}");

    // Setting it is setting the ref, to the commit a revision names.
    let (ok, _, err) = vcc(&dir.join("proj/sys"), &["base", "HEAD~1"]);
    assert!(ok && err.contains(&format!("base: {c1}")), "{err}");
    assert_eq!(git(&dir, &["rev-parse", "refs/worktree/base"]), c1);
    let (ok, out, err) = vcc(&dir, &["base"]);
    assert!(ok && out.trim() == c1 && err.is_empty(), "{out}{err}");

    // Another worktree has a base of its own.
    let wt = dir.with_file_name(format!("{}-wt", dir.file_name().unwrap().to_str().unwrap()));
    let _ = std::fs::remove_dir_all(&wt);
    git(&dir, &["worktree", "add", "-q", "--detach", wt.to_str().unwrap(), &c2]);
    let (ok, out, err) = vcc(&wt, &["base"]);
    assert!(ok && out.trim() == c2 && err.contains("unset"), "{out}{err}");
    git(&wt, &["update-ref", "refs/worktree/base", &c2]);
    assert_eq!(git(&dir, &["rev-parse", "refs/worktree/base"]), c1);

    // Mistakes.
    let (ok, _, err) = vcc(&dir, &["base", "nope"]);
    assert!(!ok && err.contains("nope doesn't name a commit"), "{err}");
    let (ok, _, err) = vcc(&dir, &["base", "a", "b"]);
    assert!(!ok && err.contains("too many arguments for base"), "{err}");
    let outside = std::env::temp_dir().join(format!("vcc-git-none-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    let (ok, _, err) = vcc(&outside, &["base"]);
    assert!(!ok && err.contains("not a git repository"), "{err}");
    git(&dir, &["worktree", "remove", "--force", wt.to_str().unwrap()]);
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&outside).unwrap();
}

#[test]
fn diff_compares_revisions() {
    let (dir, c1, c2) = repo("diff");
    let proj = dir.join("proj");
    let short = |c: &str| c[..7].to_string();
    std::fs::write(proj.join("sys/b.sys"), Q2).unwrap();
    // A new file in the working tree is a source of it, tracked or not.
    std::fs::write(proj.join("sys/c.sys"), "proc R(): int { func A(): int; R -> A(); }\n").unwrap();
    let before = state(&dir);

    // By default, the base (HEAD while it's unset) against the working tree.
    let (ok, out, err) = vcc(&proj, &["diff"]);
    assert!(ok, "{err}");
    assert!(out.starts_with(&format!("HEAD ({}) → working tree\nprocs: 1 added, 1 changed\n", short(&c2))), "{out}");
    assert!(out.contains("  ~ Q  edges ~1\n") && out.contains("  + R  nodes +2, edges +1\n"), "{out}");
    assert!(out.contains("files: 1 changed, 1 added\n  ~ sys/b.sys\n  + sys/c.sys"), "{out}");

    // Against the base once it's set, from anywhere in the project.
    git(&dir, &["update-ref", "refs/worktree/base", &c1]);
    let (ok, out, _) = vcc(&proj.join("sys"), &["diff"]);
    assert!(ok && out.starts_with(&format!("base ({}) → working tree\n", short(&c1))), "{out}");
    assert!(out.contains("  ~ P  nodes +1, edges +1\n") && out.contains("  ~ Q  edges ~1\n"), "{out}");

    // A commit against the working tree, and two commits.
    let (ok, out, _) = vcc(&proj, &["diff", "HEAD"]);
    assert!(ok && out.starts_with(&format!("HEAD ({}) → working tree\n", short(&c2))) && !out.contains("~ P"), "{out}");
    let (ok, out, _) = vcc(&proj, &["diff", "HEAD~1", "HEAD"]);
    assert_eq!(out, format!("HEAD~1 ({}) → HEAD ({})\nprocs: 1 changed\n  ~ P  nodes +1, edges +1\nfiles: 1 changed\n  ~ sys/a.sys\n", short(&c1), short(&c2)));
    assert!(ok);
    let (ok, out, _) = vcc(&proj, &["diff", "HEAD", "HEAD"]);
    assert!(ok && out.ends_with("\nno changes\n"), "{out}");

    // The document, for the visualizer: the grids with the changes marked.
    let (ok, out, _) = vcc(&proj, &["diff", "--json", "HEAD~1", "HEAD"]);
    assert!(ok && out.starts_with("{\n  \"old\": {\n    \"label\": \"HEAD~1"), "{out}");
    assert!(out.contains("\"grids\": {") && out.contains("\"label\": \"C\",\n              \"sig\": \"(n: int): int\",\n              \"diff\": \"added\""), "{out}");

    // A side that doesn't check is reported, not a failure; so is a commit
    // without the project.
    std::fs::write(proj.join("sys/b.sys"), "proc Q(n: int): int {").unwrap();
    let (ok, out, _) = vcc(&proj, &["diff"]);
    assert!(ok && out.contains("working tree doesn't check, so its procs can't be compared") && out.contains("sys/b.sys:1:"), "{out}");
    git(&dir, &["checkout", "-q", "--", "proj/sys/b.sys"]);
    let empty = git(&dir, &["commit-tree", "4b825dc642cb6eb9a060e54bf8d69288fbee4904", "-m", "empty"]);
    let (ok, out, _) = vcc(&proj, &["diff", &empty]);
    assert!(ok && out.contains(&format!("\n{} doesn't check", short(&empty))) && out.contains("there's no proj/vandorf.toml"), "{out}");

    // Mistakes.
    let (ok, _, err) = vcc(&proj, &["diff", "nope"]);
    assert!(!ok && err.contains("nope doesn't name a commit"), "{err}");
    let (ok, _, err) = vcc(&proj, &["diff", "a", "b", "c"]);
    assert!(!ok && err.contains("too many arguments for diff"), "{err}");
    let (ok, _, err) = vcc(&proj, &["diff", "--nope"]);
    assert!(!ok && err.contains("unknown flag --nope"), "{err}");

    // None of it moved HEAD or touched the index.
    std::fs::write(proj.join("sys/b.sys"), Q2).unwrap();
    assert_eq!(state(&dir), before);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Starts `vcc review` in dir: the child, and the review worktree's path,
/// the first line it prints.
fn start_review(dir: &Path) -> (std::process::Child, PathBuf) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vcc"))
        .args(["review", "-C"])
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    (child, PathBuf::from(line.trim()))
}

#[test]
fn review_is_a_throwaway_worktree_at_the_base() {
    let (dir, c1, c2) = repo("review");
    git(&dir, &["update-ref", "refs/worktree/base", &c1]);
    // A change, a new file, a deleted one and a staged one.
    std::fs::write(dir.join("proj/sys/b.sys"), Q2).unwrap();
    std::fs::write(dir.join("new.txt"), "new\n").unwrap();
    std::fs::remove_file(dir.join("README")).unwrap();
    std::fs::write(dir.join("staged.txt"), "staged\n").unwrap();
    git(&dir, &["add", "staged.txt"]);
    let before = state(&dir);

    let (mut child, review) = start_review(&dir);
    assert!(review.is_dir(), "{}", review.display());
    // HEAD and the index at the base; the files are the working tree's, so
    // every change since the base shows unstaged (new files untracked).
    assert_eq!(git(&review, &["rev-parse", "HEAD"]), c1);
    assert_eq!(git(&review, &["diff", "--cached", "--name-status"]), "");
    let status = git(&review, &["status", "--porcelain"]);
    assert_eq!(status, " D README\n M proj/sys/a.sys\n M proj/sys/b.sys\n?? new.txt\n?? staged.txt", "{status}");
    assert_eq!(std::fs::read_to_string(review.join("proj/sys/b.sys")).unwrap(), Q2);
    assert_eq!(std::fs::read_to_string(review.join("proj/sys/a.sys")).unwrap(), P2);
    // Read-only.
    assert!(std::fs::metadata(review.join("proj/sys/a.sys")).unwrap().permissions().readonly());
    assert!(std::fs::write(review.join("proj/sys/a.sys"), "x").is_err());
    // The live worktree is as it was, HEAD and index too.
    assert_eq!(state(&dir), before);
    assert_eq!(git(&dir, &["rev-parse", "HEAD"]), c2);

    // Enter removes it.
    child.stdin.take().unwrap().write_all(b"\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!review.exists());
    assert!(!git(&dir, &["worktree", "list"]).contains(review.file_name().unwrap().to_str().unwrap()));

    // One left by an interrupted review is removed by the next.
    let (mut child, stale) = start_review(&dir);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(stale.is_dir());
    let (mut child, again) = start_review(&dir);
    assert_eq!(again, stale);
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    assert!(!stale.exists());
    assert_eq!(state(&dir), before);
    std::fs::remove_dir_all(&dir).unwrap();
}
