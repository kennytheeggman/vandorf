//! Implementations of grove's interfaces (grove.sys, interiors.sys): the
//! repository, git, grove's metadata files, and the processes serving
//! interiors.

use crate::vcc::types::*;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

fn s(x: &str) -> Str {
    x.into()
}

fn diag(msg: String) -> Diag {
    Diag {
        msg: msg.into(),
        span: Span {
            at: 0,
            end: 0,
            file: 0,
        },
    }
}

fn err_str(msg: String) -> ResString {
    ResString::Err(diag(msg))
}

/// p with / separators.
fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn run_git(dir: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let err = if err.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            err
        };
        Err(if err.is_empty() {
            format!("git {} failed: {}", args.join(" "), out.status)
        } else {
            err
        })
    }
}

fn find_repo() -> Result<Repo, String> {
    let dir = match std::env::var("GROVE_REPO") {
        Ok(d) if !d.is_empty() => d.into(),
        _ => std::env::current_dir().map_err(|e| format!("no working directory: {e}"))?,
    };
    let dir = slash(&dir);
    let common = run_git(
        &dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map_err(|e| format!("grove needs a git repository: {e}"))?;
    // The main worktree is listed first; a bare repository has none.
    let list = run_git(&dir, &["worktree", "list", "--porcelain"])?;
    let first = list.split("\n\n").next().unwrap_or("");
    if first.lines().any(|l| l == "bare") {
        return Err("grove needs a repository with a working directory, not a bare one".into());
    }
    let top = first
        .lines()
        .find_map(|l| l.strip_prefix("worktree "))
        .ok_or("git listed no worktrees")?;
    let canon = |p: &str| slash(&dunce::canonicalize(p).unwrap_or_else(|_| p.into()));
    Ok(Repo {
        top: canon(top).into(),
        common: canon(&common).into(),
    })
}

/// Finds the repository grove serves, from $GROVE_REPO or else the working
/// directory, worked out once: its main worktree and its common git
/// directory, absolute.
pub fn repo(_n: i64) -> Result<ResRepo, String> {
    static REPO: OnceLock<Result<Repo, String>> = OnceLock::new();
    Ok(match REPO.get_or_init(find_repo) {
        Ok(r) => ResRepo::Ok(r.clone()),
        Err(e) => ResRepo::Err(diag(e.clone())),
    })
}

/// Runs git in dir with args, giving its output without trailing
/// whitespace, or its error output as the error.
pub fn git(dir: Str, args: List<Str>) -> Result<ResString, String> {
    let args: Vec<&str> = args.iter().map(|a| &**a).collect();
    Ok(match run_git(&dir, &args) {
        Ok(out) => ResString::Ok(out.into()),
        Err(e) => err_str(e),
    })
}

/// Makes a new idea id: eight random hex digits.
pub fn new_id(_n: i64) -> Result<Str, String> {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).map_err(|e| format!("no randomness: {e}"))?;
    Ok(b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<String>()
        .into())
}

/// Lists the names of the files in dir, sorted; none if there's no such
/// directory.
pub fn list(dir: Str) -> Result<List<Str>, String> {
    let mut names: Vec<String> = match std::fs::read_dir(&*dir) {
        Ok(es) => es
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => vec![],
    };
    names.sort();
    Ok(names.iter().map(|n| s(n)).collect())
}

/// How long a writer waits for grove's lock, and how old a lock is taken to
/// be left behind by a writer that died.
const LOCK_WAIT: Duration = Duration::from_secs(5);
const LOCK_STALE: Duration = Duration::from_secs(30);

/// grove's lock, a file made exclusively; removed when dropped.
struct Lock(std::path::PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn lock(common: &str) -> Result<Lock, String> {
    let dir = Path::new(common).join("grove");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", slash(&dir)))?;
    let path = dir.join("lock");
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(Lock(path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > LOCK_STALE);
                if stale {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err(format!("{} is held by another writer", slash(&path)));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("cannot take {}: {e}", slash(&path))),
        }
    }
}

/// Writes text to path, making its directory, through a temporary file
/// renamed into place, holding grove's lock (grove/lock in the common git
/// directory) while it does.
pub fn store(common: Str, path: Str, text: Str) -> Result<ResString, String> {
    let write = || -> Result<(), String> {
        let _held = lock(&common)?;
        let path = Path::new(&*path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", slash(dir)))?;
        }
        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        std::fs::write(&tmp, text.as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", slash(&tmp)))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("cannot write {}: {e}", slash(path))
        })
    };
    Ok(match write() {
        Ok(()) => ResString::Ok(path),
        Err(e) => err_str(e),
    })
}

// ---- Interiors ----

/// Lists the interior manifests that exist: the user's
/// ($XDG_CONFIG_HOME/grove/interiors.toml, or ~/.config/grove/interiors.toml),
/// then the repository's (grove/interiors.toml in the common git
/// directory).
pub fn manifests(common: Str) -> Result<List<Str>, String> {
    let config = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|d| !d.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")));
    let mut paths = vec![];
    if let Some(c) = config {
        paths.push(c.join("grove").join("interiors.toml"));
    }
    paths.push(Path::new(&*common).join("grove").join("interiors.toml"));
    Ok(paths
        .iter()
        .filter(|p| p.is_file())
        .map(|p| s(&slash(p)))
        .collect())
}

/// Finds a free port on 127.0.0.1.
pub fn free_port(_n: i64) -> Result<i64, String> {
    let l = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| format!("no free port: {e}"))?;
    Ok(l.local_addr().map_err(|e| e.to_string())?.port() as i64)
}

/// How long an interior has to start serving.
const START_WAIT: Duration = Duration::from_secs(30);
/// Lines of an interior's output kept, for the error when it doesn't start.
const OUTPUT_KEPT: usize = 20;

struct Served {
    child: Child,
    url: String,
}

fn served() -> &'static Mutex<HashMap<String, Served>> {
    static SERVED: OnceLock<Mutex<HashMap<String, Served>>> = OnceLock::new();
    SERVED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The first http:// or https:// URL in a line, without trailing
/// punctuation.
fn url_in(line: &str) -> Option<String> {
    let i = line.find("http://").or_else(|| line.find("https://"))?;
    let url: String = line[i..]
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | '>'))
        .collect();
    let url = url.trim_end_matches(['.', ',', ';', ')', ']']);
    (url.len() > "http://".len()).then(|| url.to_string())
}

fn start(cmd: &str, dir: &str, port: i64) -> Result<Served, String> {
    // A simple command is exec'd, so stopping the child stops the command,
    // not just a shell; one with ; & | or a newline runs in the shell.
    let mut c = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        let simple = !cmd.contains([';', '&', '|', '\n']);
        c.args([
            "-c",
            &if simple {
                format!("exec {cmd}")
            } else {
                cmd.to_string()
            },
        ]);
        c
    };
    let mut child = c
        .current_dir(dir)
        .env("PATH", super::ide_io::path_with_self())
        .envs(agent_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run `{cmd}`: {e}"))?;
    let tail = Arc::new(Mutex::new(Vec::<String>::new()));
    let (found, urls) = mpsc::channel::<String>();
    let pipes: Vec<Box<dyn Read + Send>> = vec![
        Box::new(child.stdout.take().unwrap()),
        Box::new(child.stderr.take().unwrap()),
    ];
    // Read the output for as long as the process runs, so it never blocks on
    // a full pipe.
    for pipe in pipes {
        let (tail, found) = (tail.clone(), found.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if let Some(u) = url_in(&line) {
                    let _ = found.send(u);
                }
                let mut t = tail.lock().unwrap();
                t.push(line);
                if t.len() > OUTPUT_KEPT {
                    t.remove(0);
                }
            }
        });
    }
    let deadline = Instant::now() + START_WAIT;
    let fail = |child: &mut Child, why: String| -> String {
        let _ = child.kill();
        let _ = child.wait();
        let t = tail.lock().unwrap();
        if t.is_empty() {
            why
        } else {
            format!("{why}; its output:\n{}", t.join("\n"))
        }
    };
    loop {
        if let Ok(Some(st)) = child.try_wait() {
            // Give the readers a moment to take the last of its output.
            std::thread::sleep(Duration::from_millis(50));
            return Err(fail(&mut child, format!("`{cmd}` exited ({st})")));
        }
        if port > 0 {
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port as u16));
            if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
                return Ok(Served {
                    child,
                    url: format!("http://127.0.0.1:{port}"),
                });
            }
        } else if let Ok(url) = urls.recv_timeout(Duration::from_millis(100)) {
            return Ok(Served { child, url });
        }
        if Instant::now() >= deadline {
            let what = if port > 0 {
                format!("listening on port {port}")
            } else {
                "printing a URL".to_string()
            };
            return Err(fail(
                &mut child,
                format!("`{cmd}` wasn't {what} after 30 s"),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Gives the URL of the process kept as key, first starting cmd with the
/// shell in dir if none is running, with grove's own directory first on the
/// PATH (where `ide` is built) and $GROVE_URL and $GROVE_TOKEN set, as in
/// grove's terminals: http://127.0.0.1:<port> once it accepts
/// connections when port isn't 0, else the first http:// or https:// URL it
/// prints. Waits up to 30 s.
pub fn ensure(key: Str, cmd: Str, dir: Str, port: i64) -> Result<ResString, String> {
    let mut all = served().lock().unwrap();
    if let Some(p) = all.get_mut(&*key) {
        if let Ok(None) = p.child.try_wait() {
            return Ok(ResString::Ok(s(&p.url)));
        }
        all.remove(&*key);
    }
    Ok(match start(&cmd, &dir, port) {
        Ok(p) => {
            let url = p.url.clone();
            all.insert(key.to_string(), p);
            ResString::Ok(url.into())
        }
        Err(e) => err_str(e),
    })
}

/// Stops the processes started for a worktree (kept as <name>@<worktree>),
/// giving how many there were.
pub fn stop(worktree: Str) -> Result<i64, String> {
    let suffix = format!("@{worktree}");
    let mut all = served().lock().unwrap();
    let keys: Vec<String> = all
        .keys()
        .filter(|k| k.ends_with(&suffix))
        .cloned()
        .collect();
    for k in &keys {
        if let Some(mut p) = all.remove(k) {
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
    }
    Ok(keys.len() as i64)
}

// ---- grove for agents (agents.sys) ----

/// Makes the agents' token, once per run: what grove's terminals and
/// interiors get as $GROVE_TOKEN. It's a token of its own, not the
/// session's, and only the agent routes take it (AgentGate).
pub fn agent_token(_n: i64) -> Result<Str, String> {
    static TOKEN: OnceLock<String> = OnceLock::new();
    Ok(s(TOKEN.get_or_init(|| {
        let mut b = [0u8; 16];
        let _ = getrandom::fill(&mut b);
        b.iter().map(|x| format!("{x:02x}")).collect()
    })))
}

/// $GROVE_URL and $GROVE_TOKEN for what grove starts, once it's serving.
fn agent_env() -> Vec<(String, String)> {
    let addr = crate::vcc::http::addr();
    if addr.is_empty() {
        return vec![];
    }
    vec![
        ("GROVE_URL".into(), format!("http://{addr}")),
        (
            "GROVE_TOKEN".into(),
            agent_token(0).unwrap_or_default().to_string(),
        ),
    ]
}

/// Starts the user's shell on a pty in cwd, as Term.Spawn does, with
/// $GROVE_URL (where grove is serving) and $GROVE_TOKEN (the agents'
/// token) set once grove is serving, returning the terminal's id.
pub fn spawn(cwd: Str) -> Result<ResInt, String> {
    super::ide_io::spawn_with(&cwd, &agent_env())
}

fn changes() -> &'static (Mutex<i64>, std::sync::Condvar) {
    static CHANGES: OnceLock<(Mutex<i64>, std::sync::Condvar)> = OnceLock::new();
    CHANGES.get_or_init(|| (Mutex::new(0), std::sync::Condvar::new()))
}

/// Counts a change to the tree, waking whoever waits on one; gives the new
/// count.
pub fn changed(_n: i64) -> Result<i64, String> {
    let (m, cv) = changes();
    let mut n = m.lock().unwrap();
    *n += 1;
    cv.notify_all();
    Ok(*n)
}

/// Waits up to 25 s for the count of changes to the tree to differ from v,
/// giving it then (at once if it already does).
pub fn wait(v: i64) -> Result<i64, String> {
    let (m, cv) = changes();
    let n = m.lock().unwrap();
    let (n, _) = cv
        .wait_timeout_while(n, Duration::from_secs(25), |n| *n == v)
        .unwrap();
    Ok(*n)
}

/// The canonical top of the worktree the working directory is in.
pub fn toplevel(_n: i64) -> Result<ResString, String> {
    Ok(
        match run_git(
            ".",
            &["rev-parse", "--path-format=absolute", "--show-toplevel"],
        ) {
            Ok(top) => ResString::Ok(
                slash(&dunce::canonicalize(&top).unwrap_or_else(|_| top.clone().into())).into(),
            ),
            Err(e) => err_str(format!("not in a git worktree: {e}")),
        },
    )
}

/// Tells the grove serving this repository about a change: POSTs body (a
/// JSON object) to path on $GROVE_URL with $GROVE_TOKEN, giving its
/// answer's body. An error if there's no $GROVE_URL, or grove can't be
/// reached in a few seconds, or it turns the request away.
pub fn notify(path: Str, body: Str) -> Result<ResString, String> {
    Ok(match post(&path, &body) {
        Ok(b) => ResString::Ok(b.into()),
        Err(e) => err_str(e),
    })
}

fn post(path: &str, body: &str) -> Result<String, String> {
    use std::io::Write;
    use std::net::{TcpStream, ToSocketAddrs};
    let url = std::env::var("GROVE_URL").unwrap_or_default();
    if url.is_empty() {
        return Err("no $GROVE_URL: not run in one of grove's terminals".into());
    }
    let token = std::env::var("GROVE_TOKEN").unwrap_or_default();
    let host = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("$GROVE_URL isn't an http:// URL: {url}"))?
        .trim_end_matches('/');
    let addr = host
        .to_socket_addrs()
        .map_err(|e| format!("cannot find {host}: {e}"))?
        .next()
        .ok_or_else(|| format!("cannot find {host}"))?;
    let wait = Duration::from_secs(5);
    let mut c = TcpStream::connect_timeout(&addr, wait)
        .map_err(|e| format!("cannot reach grove at {url}: {e}"))?;
    let _ = c.set_read_timeout(Some(wait));
    let _ = c.set_write_timeout(Some(wait));
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nX-Vandorf-Token: {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    c.write_all(req.as_bytes())
        .map_err(|e| format!("cannot reach grove at {url}: {e}"))?;
    let mut raw = vec![];
    c.read_to_end(&mut raw)
        .map_err(|e| format!("no answer from grove at {url}: {e}"))?;
    let raw = String::from_utf8_lossy(&raw);
    let (head, rest) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or(0);
    if status == 200 {
        return Ok(rest.to_string());
    }
    let why = rest
        .split("\"error\":")
        .nth(1)
        .map(|e| {
            e.trim()
                .trim_start_matches('"')
                .split('"')
                .next()
                .unwrap_or("")
                .to_string()
        })
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| format!("status {status}"));
    Err(format!("grove turned it away: {why}"))
}
