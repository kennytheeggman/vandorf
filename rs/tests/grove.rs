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
    assert!(tree.starts_with(&format!(r#"{{"ideas":[{{"agent":null,"behind":false,"branch":"main","fork":"","head":"{head}","id":"","name":"{name}","parent":null,"sections":[],"status":"active""#)), "{tree}");
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
    assert!(tree.contains(&format!(r#""id":"{b}","name":"{b}","parent":"{a}","rebasing":false,"sections":[{{"expanded":false,"interior":"terminal","term":0}},{{"expanded":true,"interior":"ide","term":0}}],"status":"active""#)), "{tree}");
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
    assert!(busy.status == 400 && busy.body.contains("nothing was discarded: these have changes that aren't committed: first try"), "{}", busy.body);
    git(&wt, &["checkout", "-q", "a.txt"]);
    assert_eq!(req("POST", "/api/ideas/discard", "", &token, &format!(r#"{{"id": "{a}"}}"#)).status, 200);
    assert!(!wt.exists());
    assert_eq!(git(&dir, &["rev-parse", &format!("idea/{a}")]), head);
    let tree = req("GET", "/api/ideas", "", "", "").body;
    assert!(tree.contains(&format!(r#""id":"{a}","name":"first try","parent":"","rebasing":false,"sections":[{{"expanded":false,"interior":"terminal","term":0}},{{"expanded":true,"interior":"ide","term":0}}],"status":"discarded","worktree":"""#)), "{tree}");
    // Its child went with it, and still hangs from it.
    assert!(tree.contains(&format!(r#""id":"{b}","name":"{b}","parent":"{a}","rebasing":false,"sections":[{{"expanded":false,"interior":"terminal","term":0}},{{"expanded":true,"interior":"ide","term":0}}],"status":"discarded""#)), "{tree}");
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
    assert!(tree.contains(r#""id":"byhand","name":"byhand","parent":"","rebasing":false,"sections":[],"status":"discarded""#), "{tree}");

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

    // A command longer than a line the pty takes before the shell's line
    // editor starts (1024 bytes on macOS) still runs whole in a new terminal.
    let long = "x".repeat(1500);
    let (ok, out) = grove(&wt, &env, &["run", "--idea", &b, "--new", &format!("echo {long} > long.txt")]);
    assert!(ok && out.starts_with("grove opened a terminal"), "{out}");
    assert_eq!(read_soon(&bwt.join("long.txt")), format!("{long}\n"));
    assert_eq!(terms(&bmeta()).len(), 4);

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

/// What /api/ideas says of the idea named name: (status, behind, rebasing,
/// agent state, parent).
fn idea_in(port: u16, name: &str) -> (String, bool, bool, String, String) {
    let tree = call(port, "GET", "/api/ideas", "", "").1;
    let at = tree.find(&format!(r#""name":"{name}""#)).unwrap_or_else(|| panic!("no {name} in {tree}"));
    let start = tree[..at].rfind(r#"{"agent":"#).unwrap();
    let end = at + tree[at..].find(r#""worktree":"#).unwrap();
    let i = &tree[start..end];
    let field = |k: &str| i.split(&format!(r#""{k}":"#)).nth(1).map(|v| v.split(|c| c == ',' || c == '}').next().unwrap().trim_matches('"').to_string()).unwrap_or_default();
    let state = if i.starts_with(r#"{"agent":null"#) { String::new() } else { i.split(r#""state":""#).nth(1).unwrap().split('"').next().unwrap().to_string() };
    (field("status"), field("behind") == "true", field("rebasing") == "true", state, field("parent"))
}

#[test]
fn merging_rebases_the_tree_and_agents_report() {
    let dir = repo_named("grove-merge-test");
    let common = dir.join(".git");
    let make = |name: &str, from: Option<&str>, file: &str, text: &str| -> String {
        let mut args = vec!["branch", name, "--no-terminal"];
        if let Some(f) = from { args.extend(["--from", f]); }
        let (ok, out) = grove(&dir, &[], &args);
        assert!(ok, "{out}");
        let id = out["branched idea ".len()..].split(' ').next().unwrap().to_string();
        let wt = common.join("grove/worktrees").join(&id);
        std::fs::write(wt.join(file), text).unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-qm", name]);
        id
    };
    // A, then B on A; C; and D, which changes A's file its own way.
    let a = make("A", None, "idea-a.txt", "a\n");
    let b = make("B", Some(&a), "idea-b.txt", "b\n");
    let c = make("C", None, "idea-c.txt", "c\n");
    let d = make("D", None, "idea-a.txt", "d\n");
    let wt = |id: &str| common.join("grove/worktrees").join(id);
    let (_server, port, token) = serve(&dir, &[]);

    // An idea with changes not committed can't merge; the agents' token
    // can't merge at all.
    std::fs::write(wt(&c).join("wip"), "x").unwrap();
    let (_, busy) = call(port, "POST", "/api/ideas/merge", &token, &format!(r#"{{"id": "{c}"}}"#));
    assert!(busy.contains("C has changes that aren't committed"), "{busy}");
    std::fs::remove_file(wt(&c).join("wip")).unwrap();

    // Merging B, which is on the root's head: the root fast-forwards to it,
    // B is merged, and the others are rebased onto it; D conflicts, so it's
    // left as it was and flagged.
    let (status, merged) = call(port, "POST", "/api/ideas/merge", &token, &format!(r#"{{"id": "{b}"}}"#));
    assert_eq!(status, 200, "{merged}");
    assert!(merged.contains("merged B into the root; not rebased onto it, so flagged: D (rebasing onto its parent conflicts)"), "{merged}");
    assert!(dir.join("idea-b.txt").exists() && dir.join("idea-a.txt").exists());
    assert!(!wt(&b).exists());
    assert_eq!(idea_in(port, "B").0, "merged");
    assert!(wt(&c).join("idea-b.txt").exists() && wt(&c).join("idea-c.txt").exists());
    assert_eq!(idea_in(port, "C"), ("active".into(), false, false, String::new(), String::new()));
    assert_eq!(idea_in(port, "D"), ("active".into(), true, false, "attention".into(), String::new()));
    assert_eq!(git(&wt(&d), &["status", "--porcelain"]), "");
    // A came in with B, so it's merged and left where it was.
    assert_eq!(idea_in(port, "A").0, "merged");
    let (_, again) = call(port, "POST", "/api/ideas/merge", &token, &format!(r#"{{"id": "{a}"}}"#));
    assert!(again.contains("A has nothing to merge: it's in the root already"), "{again}");

    // Agents report their status, and rebase their own idea.
    let envs = dir.join("env.txt");
    type_in_terminal(port, &token, "", &format!("env | grep ^GROVE_ | sort > {}", envs.display()));
    let text = read_soon(&envs);
    let var = |k: &str| text.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap_or_else(|| panic!("no {k} in {text}")).to_string();
    let (url, agent) = (var("GROVE_URL"), var("GROVE_TOKEN"));
    let env = [("GROVE_URL", url.as_str()), ("GROVE_TOKEN", agent.as_str())];
    assert_eq!(call(port, "POST", "/api/ideas/merge", &agent, &format!(r#"{{"id": "{c}"}}"#)).0, 403);
    let (ok, out) = grove(&wt(&c), &env, &["status", "working", "running", "the", "tests"]);
    assert_eq!((ok, out.as_str()), (true, ""));
    assert_eq!(idea_in(port, "C").3, "working");
    // A done status is unseen until a person shows the idea (the session's
    // token, not the agents'), and unseen again when the agent reports anew.
    let seen = |name: &str| {
        let tree = call(port, "GET", "/api/ideas", "", "").1;
        let at = tree.find(&format!(r#""name":"{name}""#)).unwrap();
        let start = tree[..at].rfind(r#"{"agent":"#).unwrap();
        tree[start..at].contains(r#""seen":true"#)
    };
    assert!(!seen("C"));
    assert_eq!(grove(&wt(&c), &env, &["status", "done"]).0, true);
    assert_eq!((idea_in(port, "C").3.as_str(), seen("C")), ("done", false));
    assert_eq!(call(port, "POST", "/api/ideas/seen", &agent, &format!(r#"{{"id": "{c}"}}"#)).0, 403);
    assert_eq!(call(port, "POST", "/api/ideas/seen", &token, &format!(r#"{{"id": "{c}"}}"#)).0, 200);
    assert_eq!((idea_in(port, "C").3.as_str(), seen("C")), ("done", true));
    assert_eq!(grove(&wt(&c), &env, &["status", "done"]).0, true);
    assert!(!seen("C"));
    // Only a done status can be seen.
    assert_eq!(grove(&wt(&c), &env, &["status", "working"]).0, true);
    assert_eq!(call(port, "POST", "/api/ideas/seen", &token, &format!(r#"{{"id": "{c}"}}"#)).0, 200);
    assert!(!seen("C"));
    let (ok, out) = grove(&wt(&c), &env, &["status", "busy"]);
    assert!(!ok && out.contains("grove status takes working, attention, done or clear, not busy"), "{out}");
    let (ok, out) = grove(&wt(&c), &env, &["merge"]);
    assert!(!ok && out.contains("merging is left to people"), "{out}");
    // D's rebase stops on its conflict, to be resolved where it is.
    let (ok, out) = grove(&wt(&d), &env, &["rebase"]);
    assert!(!ok && out.contains("the rebase stopped on a conflict"), "{out}");
    assert_eq!(idea_in(port, "D").2, true);
    std::fs::write(wt(&d).join("idea-a.txt"), "a and d\n").unwrap();
    git(&wt(&d), &["add", "idea-a.txt"]);
    let (ok, out) = grove(&wt(&d), &env, &["rebase", "--continue"]);
    assert!(ok && out.starts_with(&format!("rebased: {d}")), "{out}");
    assert_eq!(idea_in(port, "D"), ("active".into(), false, false, String::new(), String::new()));

    // Discarding C takes the idea under it too.
    let (ok, out) = grove(&wt(&c), &env, &["branch", "E", "--no-terminal"]);
    assert!(ok, "{out}");
    let (_, gone) = call(port, "POST", "/api/ideas/discard", &token, &format!(r#"{{"id": "{c}"}}"#));
    assert!(gone.contains(r#""ok":true"#), "{gone}");
    assert_eq!(idea_in(port, "C").0, "discarded");
    assert_eq!(idea_in(port, "E").0, "discarded");

    drop(_server);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn merging_leaves_ideas_merged_by_hand_alone() {
    let dir = repo_named("grove-handmerge-test");
    let common = dir.join(".git");
    let wt = |id: &str| common.join("grove/worktrees").join(id);
    let commit = |id: &str, file: &str, text: &str, msg: &str| {
        std::fs::write(wt(id).join(file), text).unwrap();
        git(&wt(id), &["add", "."]);
        git(&wt(id), &["commit", "-qm", msg]);
    };
    let make = |name: &str, from: Option<&str>, file: &str, text: &str| -> String {
        let mut args = vec!["branch", name, "--no-terminal"];
        if let Some(f) = from { args.extend(["--from", f]); }
        let (ok, out) = grove(&dir, &[], &args);
        assert!(ok, "{out}");
        let id = out["branched idea ".len()..].split(' ').next().unwrap().to_string();
        commit(&id, file, text, name);
        id
    };
    let head = |r: &str| git(&dir, &["rev-parse", r]);
    // E is merged into the root with git, not grove, so its metadata still
    // says active, and G, on E, isn't; H too, but with no fork known (a
    // branch made by hand); and P only up to its first commit. Then the root
    // changes their files further.
    let e = make("E", None, "idea-e.txt", "e\n");
    let g = make("G", Some(&e), "idea-g.txt", "g\n");
    let h = make("H", None, "idea-h.txt", "h\n");
    let meta = common.join("grove/ideas").join(format!("{h}.toml"));
    let text = std::fs::read_to_string(&meta).unwrap();
    let fork_line = text.lines().find(|l| l.starts_with("fork = ")).unwrap().to_string();
    std::fs::write(&meta, text.replace(&fork_line, "fork = \"\"")).unwrap();
    let p = make("P", None, "idea-p.txt", "p\n");
    commit(&p, "idea-p2.txt", "p2\n", "P2");
    for r in [format!("idea/{e}"), format!("idea/{h}"), format!("idea/{p}~1")] {
        git(&dir, &["merge", "-q", "--no-edit", &r]);
    }
    for f in ["idea-e.txt", "idea-h.txt", "idea-p.txt"] {
        std::fs::write(dir.join(f), "edited\n").unwrap();
    }
    git(&dir, &["commit", "-qam", "edit them"]);
    let (e_head, h_head) = (head(&format!("idea/{e}")), head(&format!("idea/{h}")));
    let f = make("F", None, "idea-f.txt", "f\n");
    let (_server, port, token) = serve(&dir, &[]);
    assert_eq!(idea_in(port, "E").0, "merged");
    assert_eq!(idea_in(port, "H").0, "merged");
    assert_eq!(idea_in(port, "P").0, "active");

    // Merging F restacks the tree, but E's and H's commits are in the root
    // already: replaying them would make new commits off the root, and them
    // active ideas again. G goes to the root, as if E had been merged with
    // grove, and P replays only the commit the root hasn't.
    let (status, merged) = call(port, "POST", "/api/ideas/merge", &token, &format!(r#"{{"id": "{f}"}}"#));
    assert_eq!(status, 200, "{merged}");
    assert!(merged.contains("merged F into the root; every other idea is on it now"), "{merged}");
    assert_eq!(head(&format!("idea/{e}")), e_head);
    assert_eq!(idea_in(port, "E").0, "merged");
    assert_eq!(head(&format!("idea/{h}")), h_head);
    assert_eq!(idea_in(port, "H").0, "merged");
    assert_eq!(idea_in(port, "G"), ("active".into(), false, false, String::new(), String::new()));
    assert_eq!(std::fs::read_to_string(wt(&g).join("idea-e.txt")).unwrap(), "edited\n");
    assert!(wt(&g).join("idea-f.txt").exists() && wt(&g).join("idea-g.txt").exists());
    assert_eq!(idea_in(port, "P"), ("active".into(), false, false, String::new(), String::new()));
    assert_eq!(git(&dir, &["rev-list", "--count", &format!("HEAD..idea/{p}")]), "1");
    assert_eq!(std::fs::read_to_string(wt(&p).join("idea-p.txt")).unwrap(), "edited\n");
    drop(_server);
    let _ = std::fs::remove_dir_all(&dir);
}
