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
    repo_named("grove-test")
}

fn repo_named(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{tag}-{}", std::process::id()));
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

    // The agent routes take the agents' token, not the session's; the wait
    // answers at once without a count.
    assert_eq!(req("POST", "/api/agent/notify", "", &token, r#"{"idea": ""}"#).status, 403);
    assert_eq!(req("POST", "/api/agent/notify", "", "", r#"{"idea": ""}"#).status, 403);
    assert!(req("GET", "/api/ideas/wait", "", "", "").body.starts_with(r#"{"ok":true,"v":"#));

    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs the grove executable with args in dir, with env, giving whether it
/// succeeded and what it printed (stdout, then stderr).
fn grove(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_grove"))
        .args(args)
        .current_dir(dir)
        .env_remove("GROVE_URL")
        .env_remove("GROVE_TOKEN")
        .env_remove("GROVE_REPO")
        .envs(env.iter().copied())
        .output()
        .unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr))
}

/// Waits up to 15 s for a file to have something in it.
fn read_soon(path: &Path) -> String {
    for _ in 0..150 {
        if let Ok(s) = std::fs::read_to_string(path) {
            if s.ends_with('\n') {
                return s;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("nothing in {} after 15 s", path.display());
}

/// A plain HTTP request to a running grove.
fn call(port: u16, method: &str, path: &str, token: &str, body: &str) -> (u16, String) {
    use std::io::{Read, Write};
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(std::time::Duration::from_secs(30))).unwrap();
    write!(c, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Vandorf-Token: {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut raw = String::new();
    c.read_to_string(&mut raw).unwrap();
    let status = raw.split(' ').nth(1).unwrap().parse().unwrap();
    (status, raw.split_once("\r\n\r\n").map_or("", |x| x.1).to_string())
}

/// A grove server, stopped when dropped, so a failing check doesn't leave it
/// (and its terminals) running.
struct Server(std::process::Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts grove serving dir on a free port, with env, giving it (stopped
/// when dropped), the port and the session's token. Waits up to 10 s for it.
fn serve(dir: &Path, env: &[(&str, &Path)]) -> (Server, u16, String) {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let server = Server(
        Command::new(env!("CARGO_BIN_EXE_grove"))
            .args(["--addr", &format!("127.0.0.1:{port}")])
            .current_dir(dir)
            .env_remove("GROVE_REPO")
            .envs(env.iter().copied())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut page = String::new();
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            page = call(port, "GET", "/", "", "").1;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let i = page.find("var TOKEN = '").unwrap_or_else(|| panic!("grove didn't serve on {port}")) + 13;
    let token = page[i..i + 32].to_string();
    (server, port, token)
}

/// Opens one of grove's terminals in an idea ("" for the root) and types a
/// line there.
fn type_in_terminal(port: u16, token: &str, idea: &str, line: &str) {
    let (_, term) = call(port, "POST", &format!("/api/term?idea={idea}"), token, "");
    let tid: String = term.split("\"id\":").nth(1).unwrap_or_else(|| panic!("no terminal: {term}")).chars().take_while(|c| c.is_ascii_digit()).collect();
    call(port, "POST", &format!("/api/term/input?id={tid}"), token, &format!("{line}\r"));
}

#[test]
fn terminals_find_grove_when_startup_files_reset_path() {
    // A user whose startup files set PATH outright, as `export PATH=...` in
    // a .zshrc does. zsh's and bash's terminals still have grove's directory
    // first once they're at a prompt, after those files ran; every shell's
    // have $GROVE_BIN; and the user's ZDOTDIR is theirs again.
    let grove = dunce::canonicalize(env!("CARGO_BIN_EXE_grove")).unwrap();
    let grove_dir = grove.parent().unwrap().to_str().unwrap().to_string();
    let mut ran = 0;
    for shell in ["/bin/zsh", "/bin/bash"] {
        if !Path::new(shell).exists() {
            eprintln!("no {shell}: skipped");
            continue;
        }
        let name = Path::new(shell).file_name().unwrap().to_str().unwrap();
        let dir = repo_named(&format!("grove-path-test-{name}"));
        let home = dir.join("home");
        std::fs::create_dir(&home).unwrap();
        for rc in [".zshenv", ".zshrc", ".bashrc"] {
            std::fs::write(home.join(rc), format!("export PATH=/usr/bin:/bin\necho {rc} >> {}\n", home.join("ran").display())).unwrap();
        }
        let (server, port, token) = {
            let shell = Path::new(shell);
            serve(&dir, &[("SHELL", shell), ("HOME", home.as_path()), ("ZDOTDIR", home.as_path())])
        };
        let out = dir.join("out.txt");
        type_in_terminal(port, &token, "", &format!(
            r#"printf '%s\n' "$PATH" "$(command -v grove)" "$GROVE_BIN" "${{ZDOTDIR-unset}}" "${{VANDORF_PATH_FIRST-unset}}" > {}"#,
            out.display()
        ));
        let text = read_soon(&out);
        drop(server);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5, "{shell}: {text}");
        let rcs = std::fs::read_to_string(home.join("ran")).unwrap_or_default();
        let want_rcs = if name == "zsh" { ".zshenv\n.zshrc\n" } else { ".bashrc\n" };
        assert_eq!(rcs, want_rcs, "{shell}: the user's startup files run");
        assert_eq!(lines[0], format!("{grove_dir}:/usr/bin:/bin"), "{shell}: PATH");
        assert_eq!(dunce::canonicalize(lines[1]).ok(), Some(grove.clone()), "{shell}: command -v grove gave {}", lines[1]);
        assert_eq!(dunce::canonicalize(lines[2]).ok(), Some(grove.clone()), "{shell}: $GROVE_BIN is {}", lines[2]);
        assert_eq!(lines[3], home.to_str().unwrap(), "{shell}: the user's ZDOTDIR");
        assert_eq!(lines[4], "unset", "{shell}: VANDORF_PATH_FIRST");
        let _ = std::fs::remove_dir_all(&dir);
        ran += 1;
    }
    assert!(ran > 0, "neither zsh nor bash to test with");
}

#[test]
fn agents_branch_from_the_command_line() {
    let dir = repo_named("grove-cli-test");
    let common = dir.join(".git");

    // Without a grove running: the commands work on the repository, and say
    // grove wasn't told. Discarding is left to people.
    let (ok, out) = grove(&dir, &[], &["branch", "first", "--no-terminal"]);
    assert!(ok && out.starts_with("branched idea ") && out.contains(" (first) from the root\n") && out.contains("grove wasn't told (no $GROVE_URL"), "{out}");
    let a = out["branched idea ".len()..].split(' ').next().unwrap().to_string();
    let wt = common.join("grove/worktrees").join(&a);
    let (ok, out) = grove(&wt, &[], &["current"]);
    assert!(ok && out.starts_with(&format!("id:       {a}\nname:     first\nbranch:   idea/{a}\n")), "{out}");
    let (ok, out) = grove(&wt, &[], &["list"]);
    assert!(ok && out.contains(&format!("*   {a}")), "{out}");
    let (ok, out) = grove(&wt, &[], &["discard"]);
    assert!(!ok && out.contains("discarding is left to people"), "{out}");
    let (ok, out) = grove(&wt, &[], &["bogus"]);
    assert_eq!((ok, out.as_str()), (false, "error: unknown command bogus (see grove help)\n"));
    let (ok, out) = grove(&wt, &[], &["branch", "--run", "x", "--no-terminal"]);
    assert!(!ok && out.contains("can't go with --no-terminal"), "{out}");
    let (ok, out) = grove(&wt, &[], &["help"]);
    assert!(ok && out.starts_with("usage: grove [--addr host:port]"), "{out}");
    // grove run types into grove's terminals, so it needs grove running.
    let (ok, out) = grove(&wt, &[], &["run", "--", "true"]);
    assert!(!ok && out.contains("grove run types into grove's terminals, so it needs grove running: no $GROVE_URL"), "{out}");
    let (ok, out) = grove(&wt, &[], &["run"]);
    assert!(!ok && out.contains("grove run takes a command, such as grove run -- make test, not 0"), "{out}");
    let (ok, out) = grove(&wt, &[], &["run", "--new=yes", "true"]);
    assert!(!ok && out.contains("--new takes no value"), "{out}");
    let (ok, out) = grove(&wt, &[], &["rename", "x", "--new"]);
    assert!(!ok && out.contains("grove rename takes no --new"), "{out}");

    // With grove running, its terminals carry $GROVE_URL and $GROVE_TOKEN.
    let (_server, port, token) = serve(&dir, &[]);
    let envs = dir.join("env.txt");
    type_in_terminal(port, &token, &a, &format!("env | grep ^GROVE_ | sort > {}", envs.display()));
    let text = read_soon(&envs);
    let var = |k: &str| text.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap_or_else(|| panic!("no {k} in {text}")).to_string();
    let (url, agent) = (var("GROVE_URL"), var("GROVE_TOKEN"));
    assert_eq!(url, format!("http://127.0.0.1:{port}"));
    assert_ne!(agent, token);

    // grove branch tells grove, which opens a terminal in the new idea,
    // types --run there, and keeps it in the idea's sections.
    let (_, v0) = call(port, "GET", "/api/ideas/wait", "", "");
    let env = [("GROVE_URL", url.as_str()), ("GROVE_TOKEN", agent.as_str())];
    let (ok, out) = grove(&wt, &env, &["branch", "second", "--run", "echo ran > ran.txt"]);
    assert!(ok && out.contains(&format!(" (second) from {a}\n")) && out.ends_with("grove opened a terminal in it, running: echo ran > ran.txt\n"), "{out}");
    let b = out["branched idea ".len()..].split(' ').next().unwrap().to_string();
    assert_eq!(read_soon(&common.join("grove/worktrees").join(&b).join("ran.txt")), "ran\n");
    let meta = std::fs::read_to_string(common.join(format!("grove/ideas/{b}.toml"))).unwrap();
    assert!(meta.contains(&format!("parent = \"{a}\"")) && meta.contains("interior = \"terminal\"\nexpanded = true\nterm = "), "{meta}");
    let v0: i64 = v0.split("\"v\":").nth(1).unwrap().chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap();
    let (_, v1) = call(port, "GET", &format!("/api/ideas/wait?v={v0}"), "", "");
    assert!(v1.contains(&format!("\"v\":{}", v0 + 1)), "{v1}");

    // Renaming tells grove; a wrong token is turned away, though the rename
    // is made.
    let (ok, out) = grove(&wt, &env, &["rename", "first, renamed"]);
    assert_eq!((ok, out.as_str()), (true, &*format!("renamed {a} to first, renamed\n")));
    let (ok, out) = grove(&wt, &[("GROVE_URL", &url), ("GROVE_TOKEN", "wrong")], &["rename", "again"]);
    assert!(ok && out.contains("grove turned it away: forbidden"), "{out}");

    // grove run types into the idea's first terminal section whose shell is
    // at its prompt: b's, from grove branch.
    let bwt = common.join("grove/worktrees").join(&b);
    let bmeta = || std::fs::read_to_string(common.join(format!("grove/ideas/{b}.toml"))).unwrap();
    let terms = |meta: &str| -> Vec<i64> { meta.lines().filter_map(|l| l.strip_prefix("term = ")).map(|n| n.parse().unwrap()).collect() };
    let b1 = terms(&bmeta())[0];
    let idle = |id: i64| {
        let (_, l) = call(port, "GET", "/api/term/list", &token, "");
        let ids = l.split("\"idle\":[").nth(1).unwrap_or_else(|| panic!("no idle in {l}"));
        ids[..ids.find(']').unwrap()].split(',').any(|x| x == id.to_string())
    };
    let soon = |what: &str, f: &dyn Fn() -> bool| {
        for _ in 0..100 {
            if f() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("not {what} after 10 s");
    };
    soon("idle", &|| idle(b1));
    let (ok, out) = grove(&wt, &env, &["run", "--idea", &b, "echo one > one.txt"]);
    assert_eq!((ok, out.as_str()), (true, &*format!("grove typed into a terminal of idea {b}: echo one > one.txt\n")));
    assert_eq!(read_soon(&bwt.join("one.txt")), "one\n");
    assert_eq!(terms(&bmeta()), [b1]);

    // A terminal running a program isn't typed into: the command goes to a
    // new terminal, kept in the idea's sections.
    let (ok, _) = grove(&wt, &env, &["run", "--idea", &b, "sleep 60"]);
    assert!(ok);
    soon("busy", &|| !idle(b1));
    let (ok, out) = grove(&wt, &env, &["run", "--idea", &b, "echo two > two.txt"]);
    assert_eq!((ok, out.as_str()), (true, &*format!("grove opened a terminal in idea {b}, running: echo two > two.txt\n")));
    assert_eq!(read_soon(&bwt.join("two.txt")), "two\n");
    let ts = terms(&bmeta());
    assert!(ts.len() == 2 && ts[0] == b1, "{ts:?}");
    // Once the program ends, the first section's terminal is used again.
    call(port, "POST", &format!("/api/term/input?id={b1}"), &token, "\u{3}");
    soon("idle again", &|| idle(b1));
    let (ok, out) = grove(&wt, &env, &["run", "--idea", &b, "echo three > three.txt"]);
    assert!(ok && out.starts_with("grove typed into"), "{out}");
    assert_eq!(read_soon(&bwt.join("three.txt")), "three\n");

    // --new always opens one; several arguments are quoted as words.
    let (ok, out) = grove(&wt, &env, &["run", "--idea", &b, "--new", "--", "sh", "-c", "echo \"$0\" > four.txt", "a b"]);
    assert!(ok && out.ends_with("running: sh -c 'echo \"$0\" > four.txt' 'a b'\n"), "{out}");
    assert_eq!(read_soon(&bwt.join("four.txt")), "a b\n");
    assert_eq!(terms(&bmeta()).len(), 3);

    // Never another idea's terminal: with b's terminal put in a's sections,
    // running in a (the idea here) opens a terminal of a's own.
    call(port, "POST", "/api/ideas/layout", &token, &format!(r#"{{"id": "{a}", "sections": [{{"interior": "terminal", "expanded": true, "term": {b1}}}]}}"#));
    let (ok, out) = grove(&wt, &env, &["run", "echo five > five.txt"]);
    assert_eq!((ok, out.as_str()), (true, &*format!("grove opened a terminal in idea {a}, running: echo five > five.txt\n")));
    assert_eq!(read_soon(&wt.join("five.txt")), "five\n");
    assert!(!bwt.join("five.txt").exists());

    // The run route takes the agents' token only, and an idea with a
    // worktree.
    assert_eq!(call(port, "POST", "/api/agent/run", &token, &format!(r#"{{"idea": "{b}", "cmd": "true"}}"#)).0, 403);
    let (ok, out) = grove(&wt, &env, &["run", "--idea", "nosuch", "true"]);
    assert!(!ok && out.contains("didn't run: grove turned it away: idea nosuch has no worktree"), "{out}");

    // The agents' token writes nothing else: no discarding, no layouts.
    assert_eq!(call(port, "POST", "/api/ideas/discard", &agent, &format!(r#"{{"id": "{b}"}}"#)).0, 403);
    assert_eq!(call(port, "POST", "/api/ideas/layout", &agent, &format!(r#"{{"id": "{b}", "sections": []}}"#)).0, 403);

    drop(_server);
    let _ = std::fs::remove_dir_all(&dir);
}
