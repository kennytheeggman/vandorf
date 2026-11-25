//! grove, the idea tree (../src/grove), through its server's routes, on a
//! repository made for the test: pages, ideas from git and their metadata,
//! branching, layouts, discarding and restoring, interiors' manifests, and
//! the guards on writes and terminals.

use std::path::{Path, PathBuf};
use std::process::Command;
use vcc::vcc::http;

struct Res {
    status: u16,
    body: String,
}

fn req(method: &str, path: &str, query: &str, token: &str, body: &str) -> Res {
    let r = http::Request {
        method: method.into(),
        path: path.into(),
        query: query.into(),
        headers: vec![("host".into(), "127.0.0.1".into()), ("x-vandorf-token".into(), token.into())],
        body: body.into(),
    };
    match http::handle_grove(&r) {
        Some(r) => Res { status: r.status, body: String::from_utf8(r.body).unwrap() },
        None => Res { status: 404, body: String::new() },
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// A repository with one commit, in a fresh directory.
fn repo() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("grove-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dunce::canonicalize(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-qm", "first"]);
    dir
}

fn id_of(body: &str) -> String {
    let i = body.find(r#""id":""#).unwrap_or_else(|| panic!("no id in {body}")) + 6;
    body[i..i + body[i..].find('"').unwrap()].to_string()
}

#[test]
fn ideas_are_branches_with_worktrees() {
    // grove finds its repository once per process: every check is in this
    // one test.
    let dir = repo();
    unsafe { std::env::set_var("GROVE_REPO", &dir) };
    let common = dir.join(".git");
    let head = git(&dir, &["rev-parse", "HEAD"]);

    // Pages carry the session token; writes need it.
    let page = req("GET", "/", "", "", "");
    assert_eq!(page.status, 200);
    assert!(!page.body.contains("__VANDORF_TOKEN__"));
    let i = page.body.find("var TOKEN = '").unwrap() + 13;
    let token = page.body[i..i + 32].to_string();
    assert_eq!(req("GET", "/terminal", "", "", "").status, 200);
    assert_eq!(req("GET", "/nope", "", "", "").status, 404);
    let denied = req("POST", "/api/ideas/branch", "", "wrong", r#"{"from": ""}"#);
    assert_eq!(denied.status, 403);

    // At first, only the root: the main worktree.
    let tree = req("GET", "/api/ideas", "", "", "").body;
    let name = dir.file_name().unwrap().to_str().unwrap();
    assert!(tree.starts_with(&format!(r#"{{"ideas":[{{"branch":"main","fork":"","head":"{head}","id":"","name":"{name}","parent":null,"sections":[],"status":"active""#)), "{tree}");
    assert!(tree.contains(r#""interiors":[{"mode":"builtin","name":"terminal"},{"mode":"per-worktree","name":"ide"}]"#), "{tree}");

    // Branching: a branch idea/<id>, a worktree with its base, metadata.
    let a = id_of(&req("POST", "/api/ideas/branch", "", &token, r#"{"from": "", "name": "first try"}"#).body);
    let wt = common.join("grove/worktrees").join(&a);
    assert_eq!(git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]), format!("idea/{a}"));
    assert_eq!(git(&wt, &["rev-parse", "refs/worktree/base"]), head);
    let meta = std::fs::read_to_string(common.join(format!("grove/ideas/{a}.toml"))).unwrap();
    assert!(meta.contains("name = \"first try\"\nparent = \"\"\n"), "{meta}");

    // A layout is stored, and a child idea starts with its parent's
    // interiors, without its terminals.
    let lay = req("POST", "/api/ideas/layout", "", &token, &format!(r#"{{"id": "{a}", "sections": [{{"interior": "terminal", "expanded": false, "term": 7}}, {{"interior": "ide"}}]}}"#));
    assert_eq!(lay.body, format!(r#"{{"id":"{a}","ok":true}}"#));
    let b = id_of(&req("POST", "/api/ideas/branch", "", &token, &format!(r#"{{"from": "{a}"}}"#)).body);
    let tree = req("GET", "/api/ideas", "", "", "").body;
    assert!(tree.contains(&format!(r#""id":"{b}","name":"{b}","parent":"{a}","sections":[{{"expanded":false,"interior":"terminal","term":0}},{{"expanded":true,"interior":"ide","term":0}}],"status":"active""#)), "{tree}");
    for (body, err) in [
        (r#"{"id": "../x", "sections": []}"#, "invalid idea id: ../x"),
        (r#"{"id": "nobranch", "sections": []}"#, "no such idea: nobranch"),
        (r#"{"id": "", "sections": [{"expanded": true}]}"#, "sections[0]: missing interior, a string"),
    ] {
        let r = req("POST", "/api/ideas/layout", "", &token, body);
        assert_eq!((r.status, r.body), (400, format!(r#"{{"error":"{err}","ok":false}}"#)));
    }

    // Discarding: git refuses a worktree with changes; then the worktree
    // goes, the branch stays, and it can be restored.
    std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
    let busy = req("POST", "/api/ideas/discard", "", &token, &format!(r#"{{"id": "{a}"}}"#));
    assert!(busy.status == 400 && busy.body.contains("contains modified or untracked files"), "{}", busy.body);
    git(&wt, &["checkout", "-q", "a.txt"]);
    assert_eq!(req("POST", "/api/ideas/discard", "", &token, &format!(r#"{{"id": "{a}"}}"#)).status, 200);
    assert!(!wt.exists());
    assert_eq!(git(&dir, &["rev-parse", &format!("idea/{a}")]), head);
    let tree = req("GET", "/api/ideas", "", "", "").body;
    assert!(tree.contains(&format!(r#""id":"{a}","name":"first try","parent":"","sections":[{{"expanded":false,"interior":"terminal","term":0}},{{"expanded":true,"interior":"ide","term":0}}],"status":"discarded","worktree":"""#)), "{tree}");
    // Its child still hangs from it.
    assert!(tree.contains(&format!(r#""parent":"{a}""#)));
    let again = req("POST", "/api/ideas/discard", "", &token, &format!(r#"{{"id": "{a}"}}"#));
    assert!(again.body.contains("has no worktree"), "{}", again.body);
    assert_eq!(req("POST", "/api/ideas/restore", "", &token, &format!(r#"{{"id": "{a}"}}"#)).status, 200);
    assert_eq!(git(&wt, &["rev-parse", "refs/worktree/base"]), head);
    let twice = req("POST", "/api/ideas/restore", "", &token, &format!(r#"{{"id": "{a}"}}"#));
    assert_eq!(twice.body, format!(r#"{{"error":"idea {a} isn't discarded","ok":false}}"#));
    let root = req("POST", "/api/ideas/discard", "", &token, r#"{"id": ""}"#);
    assert_eq!(root.body, r#"{"error":"the repository itself can't be discarded","ok":false}"#);

    // Renaming, the root too.
    assert_eq!(req("POST", "/api/ideas/rename", "", &token, r#"{"id": "", "name": "trunk"}"#).status, 200);
    assert!(req("GET", "/api/ideas", "", "", "").body.contains(r#""id":"","name":"trunk""#));

    // A branch made outside grove is an idea too, from the root.
    git(&dir, &["branch", "idea/byhand"]);
    let tree = req("GET", "/api/ideas", "", "", "").body;
    assert!(tree.contains(r#""id":"byhand","name":"byhand","parent":"","sections":[],"status":"discarded""#), "{tree}");

    // Interiors from the repository's manifest: they can replace the IDE's
    // entry, not the terminal's, and add their own after.
    let manifest = common.join("grove/interiors.toml");
    std::fs::write(&manifest, "[interior.ide]\nmode = \"shared\"\nstart = \"ide --addr 127.0.0.1:{port}\"\n[interior.terminal]\nstart = \"no\"\n[interior.docs]\nstart = \"serve\"\n").unwrap();
    let tree = req("GET", "/api/ideas", "", "", "").body;
    assert!(tree.contains(r#""interiors":[{"mode":"builtin","name":"terminal"},{"mode":"shared","name":"ide"},{"mode":"per-worktree","name":"docs"}]"#), "{tree}");
    std::fs::write(&manifest, "[interior.ide]\nmode = \"sometimes\"\nstart = \"ide\"\n").unwrap();
    let bad = req("GET", "/api/ideas", "", "", "").body;
    assert!(bad.ends_with(r#"grove/interiors.toml: interior.ide.mode must be \"shared\" or \"per-worktree\", found \"sometimes\"","ok":false}"#), "{bad}");
    std::fs::remove_file(&manifest).unwrap();

    // Called in-process, not listening on a loopback address: no terminals,
    // and no interiors started.
    let term = req("POST", "/api/term", &format!("idea={a}"), &token, "");
    assert_eq!(term.body, r#"{"error":"terminals are off: the IDE isn't listening on a loopback address","ok":false}"#);
    let open = req("POST", "/api/interior/open", "", &token, r#"{"idea": "", "interior": "terminal"}"#);
    assert_eq!(open.status, 400);

    let _ = std::fs::remove_dir_all(&dir);
}
