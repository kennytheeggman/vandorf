//! grove's leaves (grove.sys, ideas.sys, interiors.sys): pure functions over
//! ideas' metadata, git's output, interiors' manifests and the JSON the
//! pages read.

use crate::vcc::types::*;
use std::collections::{BTreeMap, BTreeSet};

fn s(x: &str) -> Str {
    x.into()
}

fn no_span(msg: String) -> Diag {
    Diag {
        msg: msg.into(),
        span: Span {
            at: 0,
            end: 0,
            file: 0,
        },
    }
}

fn obj(fs: Vec<(&str, Json)>) -> Json {
    Json::Obj(
        fs.into_iter()
            .map(|(k, v)| JsonField {
                key: k.into(),
                value: v,
            })
            .collect(),
    )
}

fn jstr(x: &str) -> Json {
    Json::Str(x.into())
}

fn field<'a>(j: &'a Json, key: &str) -> Option<&'a Json> {
    match j {
        Json::Obj(fs) => fs.iter().find(|f| &*f.key == key).map(|f| &f.value),
        _ => None,
    }
}

/// Makes an error with no source position.
pub fn grove_err(msg: Str) -> Result<Diag, String> {
    Ok(no_span(msg.to_string()))
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Whether id is a usable idea id: letters, digits, '-', '_' and '.', not
/// starting with '.', at most 64 of them.
pub fn valid_id(id: Str) -> Result<bool, String> {
    Ok(is_valid_id(&id))
}

/// The path of an idea's metadata: grove/ideas/<id>.toml in the common git
/// directory, or grove/root.toml for the root ("").
pub fn meta_path(common: Str, id: Str) -> Result<Str, String> {
    Ok(if id.is_empty() {
        format!("{common}/grove/root.toml").into()
    } else {
        format!("{common}/grove/ideas/{id}.toml").into()
    })
}

/// The path grove gives a new idea's worktree: grove/worktrees/<id> in the
/// common git directory.
pub fn worktree_path(common: Str, id: Str) -> Result<Str, String> {
    Ok(format!("{common}/grove/worktrees/{id}").into())
}

/// A worktree in `git worktree list --porcelain`: its path, HEAD and branch
/// (the full ref, "" when detached).
struct Worktree {
    path: String,
    head: String,
    branch: String,
}

fn worktrees(porcelain: &str) -> Vec<Worktree> {
    let mut out = vec![];
    for block in porcelain.split("\n\n") {
        let mut w = Worktree {
            path: String::new(),
            head: String::new(),
            branch: String::new(),
        };
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                w.path = p.to_string();
            } else if let Some(h) = line.strip_prefix("HEAD ") {
                w.head = h.to_string();
            } else if let Some(b) = line.strip_prefix("branch ") {
                w.branch = b.to_string();
            }
        }
        if !w.path.is_empty() {
            out.push(w);
        }
    }
    out
}

/// Finds the worktree that has branch idea/<id> checked out, in the output of
/// `git worktree list --porcelain`, or else grove's worktree for it
/// (grove/worktrees/<id>) with a detached HEAD, as while a rebase is
/// stopped; "" if there's none.
pub fn worktree_of(porcelain: Str, id: Str) -> Result<Str, String> {
    let wts = worktrees(&porcelain);
    Ok(find_wt(&wts, &id).map_or(s(""), |w| w.path.as_str().into()))
}

/// The worktree with idea/<id> checked out, or else grove's worktree for it
/// (grove/worktrees/<id>) with a detached HEAD, as while a rebase is stopped.
fn find_wt<'a>(wts: &'a [Worktree], id: &str) -> Option<&'a Worktree> {
    let want = format!("refs/heads/idea/{id}");
    let tail = format!("/grove/worktrees/{id}");
    wts.iter().find(|w| w.branch == want).or_else(|| {
        wts.iter()
            .find(|w| w.branch.is_empty() && w.path.ends_with(&tail))
    })
}

/// The idea whose grove worktree (grove/worktrees/<id>) path is, or "".
pub fn id_of_path(path: Str) -> Result<Str, String> {
    Ok(match path.rsplit_once("/grove/worktrees/") {
        Some((_, id)) if is_valid_id(id) => s(id),
        _ => s(""),
    })
}

fn new(id: &str) -> Idea {
    Idea {
        id: s(id),
        name: s(id),
        parent: s(""),
        fork: s(""),
        status: s("active"),
        sections: List::default(),
    }
}

/// Reads an idea's metadata, filling what's missing (a name is the id, a
/// status is active).
pub fn parse_idea(text: Str, id: Str) -> Result<ResIdea, String> {
    let file = if id.is_empty() {
        "root.toml".to_string()
    } else {
        format!("{id}.toml")
    };
    let t: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => return Ok(ResIdea::Err(no_span(format!("{file}: {e}")))),
    };
    let str_of = |t: &toml::Table, k: &str, d: &str| -> Str {
        t.get(k).and_then(|v| v.as_str()).unwrap_or(d).into()
    };
    let mut i = new(&id);
    i.name = str_of(&t, "name", &id);
    i.parent = str_of(&t, "parent", "");
    i.fork = str_of(&t, "fork", "");
    i.status = str_of(&t, "status", "active");
    if let Some(xs) = t.get("sections").and_then(|v| v.as_array()) {
        i.sections = xs
            .iter()
            .filter_map(|x| x.as_table())
            .filter_map(|x| {
                Some(Section {
                    interior: x.get("interior")?.as_str()?.into(),
                    expanded: x.get("expanded").and_then(|v| v.as_bool()).unwrap_or(true),
                    term: x.get("term").and_then(|v| v.as_integer()).unwrap_or(0),
                })
            })
            .collect();
    }
    Ok(ResIdea::Ok(i))
}

/// A TOML basic string.
fn quoted(x: &str) -> String {
    let mut out = String::from("\"");
    for c in x.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Writes an idea's metadata as TOML.
pub fn idea_toml(i: Idea) -> Result<Str, String> {
    let mut out = String::from("# grove's metadata for an idea: not committed, not pushed.\n");
    for (k, v) in [
        ("id", &i.id),
        ("name", &i.name),
        ("parent", &i.parent),
        ("fork", &i.fork),
        ("status", &i.status),
    ] {
        out.push_str(&format!("{k} = {}\n", quoted(v)));
    }
    for x in i.sections.iter() {
        out.push_str(&format!(
            "\n[[sections]]\ninterior = {}\nexpanded = {}\nterm = {}\n",
            quoted(&x.interior),
            x.expanded,
            x.term
        ));
    }
    Ok(out.into())
}

/// The metadata of an idea that has none stored: named after its id, branched
/// from the root, active, with no sections.
pub fn new_idea(id: Str) -> Result<Idea, String> {
    Ok(new(&id))
}

/// The ids of the ideas: those with metadata (file names <id>.toml) and those
/// with a branch (lines "refs/heads/idea/<id> <commit>" from `git
/// for-each-ref`), sorted, each once, leaving out ones that aren't valid.
pub fn ids_of(files: List<Str>, branches: Str) -> Result<List<Str>, String> {
    let mut ids = BTreeSet::new();
    for f in files.iter() {
        if let Some(id) = f.strip_suffix(".toml") {
            ids.insert(id.to_string());
        }
    }
    for line in branches.lines() {
        if let Some(id) = line
            .split(' ')
            .next()
            .and_then(|r| r.strip_prefix("refs/heads/idea/"))
        {
            ids.insert(id.to_string());
        }
    }
    Ok(ids
        .into_iter()
        .filter(|id| is_valid_id(id))
        .map(|id| id.into())
        .collect())
}

/// Reads a string field, or d if it's missing or not a string.
pub fn str_or(j: Json, key: Str, d: Str) -> Result<Str, String> {
    Ok(match field(&j, &key) {
        Some(Json::Str(x)) => x.clone(),
        _ => d,
    })
}

/// Reads the "sections" array of a request: each an object with "interior",
/// and optionally "expanded" and "term".
pub fn parse_sections(j: Json) -> Result<ResSectionList, String> {
    let bad = |msg: &str| Ok(ResSectionList::Err(no_span(msg.into())));
    let Some(Json::Arr(xs)) = field(&j, "sections") else {
        return bad("missing sections, an array");
    };
    let mut out = vec![];
    for (i, x) in xs.iter().enumerate() {
        let Some(Json::Str(interior)) = field(x, "interior") else {
            return bad(&format!("sections[{i}]: missing interior, a string"));
        };
        let expanded = match field(x, "expanded") {
            None => true,
            Some(Json::Bool(b)) => *b,
            Some(_) => return bad(&format!("sections[{i}]: expanded isn't a bool")),
        };
        let term = match field(x, "term") {
            None | Some(Json::Null) => 0,
            Some(Json::Num(n)) => *n,
            Some(_) => return bad(&format!("sections[{i}]: term isn't an integer")),
        };
        out.push(Section {
            interior: interior.clone(),
            expanded,
            term,
        });
    }
    Ok(ResSectionList::Ok(out.into()))
}

/// Sections for a new idea branched from one with these: the same interiors,
/// expanded as they were, with no terminals yet.
pub fn fresh_sections(xs: List<Section>) -> Result<List<Section>, String> {
    Ok(xs
        .iter()
        .map(|x| Section {
            term: 0,
            ..x.clone()
        })
        .collect())
}

/// Maps a path to the grove page served there: "grove" for /, "grove-term"
/// for /terminal, "" for any other.
pub fn grove_page(path: Str) -> Result<Str, String> {
    Ok(s(match &*path {
        "/" => "grove",
        "/terminal" => "grove-term",
        _ => "",
    }))
}

fn sections_json(xs: &List<Section>) -> Json {
    Json::Arr(
        xs.iter()
            .map(|x| {
                obj(vec![
                    ("interior", Json::Str(x.interior.clone())),
                    ("expanded", Json::Bool(x.expanded)),
                    ("term", Json::Num(x.term)),
                ])
            })
            .collect(),
    )
}

/// Builds the tree's JSON: the repository, the root and each idea that has a
/// branch (its worktree, head and sections; one whose parent is gone hangs
/// from the root; one with no worktree is discarded), and the interiors that
/// can be opened.
/// An idea is merged when its metadata says so, or when its branch has
/// commits of its own (its head isn't its fork, or with no fork, the
/// root's head) and they're all in the
/// root's head (merged: `git for-each-ref --merged HEAD` lines). It's behind
/// when it's active and its fork isn't its parent's head. agents: what
/// agents said of each idea.
pub fn tree_json(
    repo: Repo,
    worktrees: Str,
    branches: Str,
    merged: Str,
    root: Idea,
    ideas: List<Idea>,
    interiors: List<Interior>,
    agents: Map<Str, AgentStatus>,
) -> Result<Json, String> {
    let in_root: BTreeSet<&str> = merged
        .lines()
        .filter_map(|l| l.trim().strip_prefix("refs/heads/idea/"))
        .collect();
    let agent = |id: &str| match agents.get(&s(id)) {
        Some(a) if !a.state.is_empty() => obj(vec![
            ("state", Json::Str(a.state.clone())),
            ("message", Json::Str(a.message.clone())),
        ]),
        _ => Json::Null,
    };
    let wts = self::worktrees(&worktrees);
    let heads: BTreeMap<&str, &str> = branches
        .lines()
        .filter_map(|l| {
            let (r, h) = l.split_once(' ')?;
            Some((r.strip_prefix("refs/heads/idea/")?, h))
        })
        .collect();
    let known: BTreeSet<&str> = ideas
        .iter()
        .filter(|i| heads.contains_key(&*i.id))
        .map(|i| &*i.id)
        .collect();
    let main = wts.first();
    let dir_name = repo.top.rsplit('/').next().unwrap_or("").to_string();
    let mut out = vec![obj(vec![
        ("id", jstr("")),
        (
            "name",
            jstr(if root.name.is_empty() {
                &dir_name
            } else {
                &root.name
            }),
        ),
        ("parent", Json::Null),
        ("fork", jstr("")),
        ("status", jstr("active")),
        (
            "branch",
            jstr(main.map_or("", |w| {
                w.branch.strip_prefix("refs/heads/").unwrap_or(&w.branch)
            })),
        ),
        ("head", jstr(main.map_or("", |w| &w.head))),
        ("worktree", Json::Str(repo.top.clone())),
        ("sections", sections_json(&root.sections)),
        ("behind", Json::Bool(false)),
        ("agent", agent("")),
    ])];
    let root_head = main.map_or("", |w| &*w.head);
    for i in ideas.iter().filter(|i| known.contains(&*i.id)) {
        let branch = format!("refs/heads/idea/{}", i.id);
        let wt = find_wt(&wts, &i.id);
        let parent = if known.contains(&*i.parent) && i.parent != i.id {
            &*i.parent
        } else {
            ""
        };
        let head = heads.get(&*i.id).copied().unwrap_or("");
        // Commits of its own: past its fork, or with no fork known (a branch
        // made by hand), past the root's head.
        let own = !head.is_empty()
            && head
                != if i.fork.is_empty() {
                    root_head
                } else {
                    &*i.fork
                };
        let status = if &*i.status == "merged" || (in_root.contains(&*i.id) && own) {
            "merged"
        } else if wt.is_some() {
            "active"
        } else {
            "discarded"
        };
        let parent_head = if parent.is_empty() {
            root_head
        } else {
            heads.get(parent).copied().unwrap_or("")
        };
        let behind = status == "active" && !parent_head.is_empty() && parent_head != &*i.fork;
        out.push(obj(vec![
            ("id", Json::Str(i.id.clone())),
            ("name", Json::Str(i.name.clone())),
            ("parent", jstr(parent)),
            ("fork", Json::Str(i.fork.clone())),
            ("status", jstr(status)),
            ("branch", jstr(&branch["refs/heads/".len()..])),
            ("head", jstr(head)),
            ("worktree", jstr(wt.map_or("", |w| &w.path))),
            ("sections", sections_json(&i.sections)),
            ("behind", Json::Bool(behind)),
            (
                "rebasing",
                Json::Bool(wt.is_some_and(|w| w.branch.is_empty())),
            ),
            ("agent", agent(&i.id)),
        ]));
    }
    Ok(obj(vec![
        ("ok", Json::Bool(true)),
        (
            "repo",
            obj(vec![
                ("top", Json::Str(repo.top.clone())),
                ("name", jstr(&dir_name)),
            ]),
        ),
        ("ideas", Json::Arr(out.into())),
        (
            "interiors",
            Json::Arr(
                interiors
                    .iter()
                    .map(|x| {
                        obj(vec![
                            ("name", Json::Str(x.name.clone())),
                            ("mode", Json::Str(x.mode.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]))
}

/// Answers with {"ok": true, key: value}, or the error.
pub fn answer(r: ResString, key: Str) -> Result<HttpResponse, String> {
    match r {
        ResString::Ok(v) => {
            super::ide_leaf::api(obj(vec![("ok", Json::Bool(true)), (&key, Json::Str(v))]))
        }
        ResString::Err(e) => super::ide_leaf::api_err(e.msg),
    }
}

/// a, or d if a is "".
pub fn or_str(a: Str, d: Str) -> Result<Str, String> {
    Ok(if a.is_empty() { d } else { a })
}

pub fn unwrap_sections(r: ResSectionList) -> Result<List<Section>, String> {
    match r {
        ResSectionList::Ok(xs) => Ok(xs),
        ResSectionList::Err(e) => Err(e.msg.to_string()),
    }
}

// ---- Interiors ----

/// The interiors grove has itself: the terminal, served by grove, and
/// vandorf's IDE (`ide`), one per idea.
pub fn built_ins(_n: i64) -> Result<List<Interior>, String> {
    Ok(vec![
        Interior {
            name: s("terminal"),
            mode: s("builtin"),
            start: s(""),
            open: s("/terminal?idea={idea}"),
        },
        Interior {
            name: s("ide"),
            mode: s("per-worktree"),
            start: s("ide --addr 127.0.0.1:{port}"),
            open: s("{url}"),
        },
    ]
    .into())
}

/// Reads a manifest's [interior.<name>] tables; an error names the file.
pub fn parse_interiors(text: Str, file: Str) -> Result<ResInteriorList, String> {
    let bad = |msg: String| Ok(ResInteriorList::Err(no_span(format!("{file}: {msg}"))));
    let t: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => return bad(e.to_string()),
    };
    let mut out = vec![];
    for (k, v) in &t {
        if k != "interior" {
            return bad(format!("unknown table `{k}`, expected `interior`"));
        }
        let Some(tables) = v.as_table() else {
            return bad("`interior` must be a table of interiors, [interior.<name>]".into());
        };
        for (name, x) in tables {
            let Some(x) = x.as_table() else {
                return bad(format!("interior.{name} must be a table"));
            };
            let get = |key: &str| -> Result<Option<&str>, String> {
                match x.get(key) {
                    None => Ok(None),
                    Some(v) => v
                        .as_str()
                        .map(Some)
                        .ok_or_else(|| format!("interior.{name}.{key} must be a string")),
                }
            };
            for key in x.keys() {
                if !matches!(key.as_str(), "mode" | "start" | "open") {
                    return bad(format!(
                        "interior.{name}: unknown key `{key}`, expected `mode`, `start` or `open`"
                    ));
                }
            }
            let (mode, start, open) = match (get("mode"), get("start"), get("open")) {
                (Ok(m), Ok(st), Ok(o)) => (m.unwrap_or("per-worktree"), st, o.unwrap_or("{url}")),
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return bad(e),
            };
            if mode != "shared" && mode != "per-worktree" {
                return bad(format!(
                    "interior.{name}.mode must be \"shared\" or \"per-worktree\", found {mode:?}"
                ));
            }
            let Some(start) = start else {
                return bad(format!(
                    "interior.{name} needs `start`, the command that serves it"
                ));
            };
            out.push(Interior {
                name: s(name),
                mode: s(mode),
                start: s(start),
                open: s(open),
            });
        }
    }
    Ok(ResInteriorList::Ok(out.into()))
}

/// The interiors of a, with b's replacing a's of the same name, and b's
/// others after them. The terminal can't be replaced.
pub fn merge_interiors(a: List<Interior>, b: List<Interior>) -> Result<List<Interior>, String> {
    let mut out: Vec<Interior> = a.to_vec();
    for x in b.iter() {
        match out.iter_mut().find(|y| y.name == x.name) {
            Some(y) if &*y.mode == "builtin" => {}
            Some(y) => *y = x.clone(),
            None => out.push(x.clone()),
        }
    }
    Ok(out.into())
}

pub fn find_interior(xs: List<Interior>, name: Str) -> Result<MaybeInterior, String> {
    Ok(xs
        .iter()
        .find(|x| x.name == name)
        .map_or(MaybeInterior::None, |x| MaybeInterior::Some(x.clone())))
}

/// Whether a start command has a {port} for grove to fill.
pub fn has_port(cmd: Str) -> Result<bool, String> {
    Ok(cmd.contains("{port}"))
}

fn sh_quote(x: &str) -> String {
    if !x.is_empty()
        && x.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-' | ':' | '@' | '=')
        })
    {
        x.to_string()
    } else {
        format!("'{}'", x.replace('\'', "'\\''"))
    }
}

fn url_encode(x: &str) -> String {
    let mut out = String::new();
    for b in x.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Fills a template's {name} placeholders from vars, leaving unknown ones as
/// they are. Each value but {url}'s is quoted for the shell ("sh") or
/// percent-encoded for a URL ("url").
pub fn fill(template: Str, vars: Map<Str, Str>, quote: Str) -> Result<Str, String> {
    let mut out = String::new();
    let mut rest = &*template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('}') {
            Some(j) if vars.contains_key(&after[..j]) => {
                let key = &after[..j];
                let v = &vars[key];
                out.push_str(&match (&*quote, key) {
                    (_, "url") => v.to_string(),
                    ("sh", _) => sh_quote(v),
                    ("url", _) => url_encode(v),
                    _ => v.to_string(),
                });
                rest = &after[j + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Ok(out.into())
}

/// The placeholders of an idea's interior: {worktree}, {idea}, {base} and
/// {repo}.
pub fn vars(worktree: Str, idea: Str, base: Str, repo: Str) -> Result<Map<Str, Str>, String> {
    Ok([
        (s("worktree"), worktree),
        (s("idea"), idea),
        (s("base"), base),
        (s("repo"), repo),
    ]
    .into_iter()
    .collect())
}

// ---- grove for agents (agents.sys) ----

const USAGE: &str =
    "usage: grove [--addr host:port]     serve the idea tree (default 127.0.0.1:8090)
       grove <command> [args]

Commands work in the idea whose worktree they're run in (\"this idea\"; the
repository's own worktree is the root):

  list                       the ideas as a tree; * marks this idea
  current                    this idea: its id, name, branch and worktree
  branch [name] [--from <idea>] [--run <cmd>] [--no-terminal]
                             a new idea branched from this one (or --from's;
                             --from \"\" is the root), at its head, with a
                             worktree of its own. grove opens a terminal in
                             it and types --run's command there, such as
                             --run 'claude \"try a cache\"'.
  rename <name> [--idea <idea>]
                             renames this idea (or --idea)
  restore <idea>             gives a discarded idea its worktree back
  run [--idea <idea>] [--new] [--] <cmd...>
                             types cmd into a terminal of this idea (or
                             --idea's): its first terminal section whose
                             shell is at its prompt, else a new terminal,
                             kept in its sections; --new always opens one.
                             A terminal running a program (an agent, an
                             editor, this command itself) is never typed
                             into. One argument is a shell command line
                             (grove run 'make test && echo ok'); several are
                             quoted as words. Needs grove running.
  status <working|attention|done|clear> [message] [--idea <idea>]
                             tells grove what the agent here is doing, shown
                             on its node: working (a spinner), attention
                             (amber), done (green); clear forgets it. Quiet
                             when grove hears it, for agents' hooks, e.g.
                             [ -n \"$GROVE_URL\" ] && \"$GROVE_BIN\" status done
  rebase [--idea <idea>] [--continue]
                             rebases this idea onto its parent's head. On a
                             conflict it stops for you to resolve it; then
                             git add the files and grove rebase --continue.
                             An idea must be on the root's head to be merged.
  help                       this

Discarding and merging are left to people, in grove's tree. Each command works on the
repository itself, then tells grove ($GROVE_URL, with $GROVE_TOKEN, both
set in grove's terminals) so its tree catches up at once.

grove's terminals have grove's directory first on PATH (put back after zsh's
and bash's startup files), and $GROVE_BIN, grove's own executable: scripts
and agents can run \"$GROVE_BIN\" branch ... wherever PATH has been reset.";

/// Reads grove's command line: a command first (help for -h, --help or
/// none), then its arguments and flags, as `--flag value` or `--flag=value`;
/// err says what's wrong, for an unknown command or flag or the wrong number
/// of arguments.
pub fn parse_cli(argv: List<Str>) -> Result<CliArgs, String> {
    let mut a = CliArgs {
        cmd: s("help"),
        args: List::default(),
        from: s(""),
        has_from: false,
        idea: s(""),
        has_idea: false,
        run: s(""),
        terminal: true,
        fresh: false,
        cont: false,
        err: s(""),
    };
    let mut it = argv.iter().map(|x| x.to_string());
    match it.next().as_deref() {
        None | Some("-h" | "--help" | "help") => return Ok(a),
        Some(c) => a.cmd = s(c),
    }
    let mut args = vec![];
    let fail = |mut a: CliArgs, msg: String| {
        a.err = msg.into();
        Ok(a)
    };
    while let Some(x) = it.next() {
        if x == "-h" || x == "--help" {
            a.cmd = s("help");
            return Ok(a);
        }
        // `grove run`'s command is the rest, from its first word on.
        if x == "--" || (&*a.cmd == "run" && !args.is_empty()) {
            if x != "--" {
                args.push(x);
            }
            args.extend(it.by_ref());
            break;
        }
        let Some(flag) = x.strip_prefix("--") else {
            args.push(x);
            continue;
        };
        let (name, inline) = match flag.split_once('=') {
            Some((n, v)) => (n.to_string(), Some(v.to_string())),
            None => (flag.to_string(), None),
        };
        if matches!(name.as_str(), "no-terminal" | "new" | "continue") {
            if inline.is_some() {
                return fail(a, format!("--{name} takes no value"));
            }
            match name.as_str() {
                "new" => a.fresh = true,
                "continue" => a.cont = true,
                _ => a.terminal = false,
            }
            continue;
        }
        if !matches!(name.as_str(), "from" | "idea" | "run") {
            return fail(a, format!("unknown flag --{name}"));
        }
        let Some(v) = inline.or_else(|| it.next()) else {
            return fail(a, format!("--{name} needs a value"));
        };
        match name.as_str() {
            "from" => (a.from, a.has_from) = (v.into(), true),
            "idea" => (a.idea, a.has_idea) = (v.into(), true),
            _ => a.run = v.into(),
        }
    }
    let (n, min, max, flags): (usize, usize, usize, &[&str]) = match &*a.cmd {
        "list" | "current" => (args.len(), 0, 0, &[]),
        "branch" => (args.len(), 0, 1, &["from", "run", "no-terminal"]),
        "rename" => (args.len(), 1, 1, &["idea"]),
        "restore" => (args.len(), 1, 1, &[]),
        "run" => (args.len(), 1, usize::MAX, &["idea", "new"]),
        "status" => (args.len(), 1, usize::MAX, &["idea"]),
        "rebase" => (args.len(), 0, 0, &["idea", "continue"]),
        "discard" | "merge" => return Ok(a),
        c => return fail(a.clone(), format!("unknown command {c}")),
    };
    let used = [
        ("from", a.has_from),
        ("idea", a.has_idea),
        ("run", !a.run.is_empty()),
        ("no-terminal", !a.terminal),
        ("new", a.fresh),
        ("continue", a.cont),
    ];
    if let Some((f, _)) = used.iter().find(|(f, on)| *on && !flags.contains(f)) {
        return fail(a.clone(), format!("grove {} takes no --{f}", a.cmd));
    }
    if !a.terminal && !a.run.is_empty() {
        return fail(a, "--run types its command into the new idea's terminal, so it can't go with --no-terminal".into());
    }
    if n < min || n > max {
        let want = match (min, max) {
            (0, 0) => "no arguments".to_string(),
            (0, 1) => "at most one argument, the name".to_string(),
            (1, usize::MAX) if &*a.cmd == "status" => {
                "a state: working, attention, done or clear".to_string()
            }
            (1, usize::MAX) => "a command, such as grove run -- make test".to_string(),
            _ => "one argument".to_string(),
        };
        return fail(a.clone(), format!("grove {} takes {want}, not {n}", a.cmd));
    }
    if &*a.cmd == "rename" && args[0].is_empty() {
        return fail(a, "a name can't be empty".into());
    }
    if &*a.cmd == "status"
        && !matches!(args[0].as_str(), "working" | "attention" | "done" | "clear")
    {
        return fail(
            a.clone(),
            format!(
                "grove status takes working, attention, done or clear, not {}",
                args[0]
            ),
        );
    }
    if &*a.cmd == "run" && args.iter().all(|x| x.trim().is_empty()) {
        return fail(a, "grove run takes a command, not an empty one".into());
    }
    a.args = args.into_iter().map(Str::from).collect();
    Ok(a)
}

/// grove's usage, for `grove help`.
pub fn cli_usage(_n: i64) -> Result<Str, String> {
    Ok(s(USAGE))
}

/// Fails with msg, which grove prints as its error.
pub fn cli_fail(msg: Str) -> Result<(), String> {
    Err(msg.to_string())
}

/// The id of the idea a branch is checked out for: idea/<id> gives id; an
/// error for any other branch.
pub fn idea_of_branch(branch: Str) -> Result<ResString, String> {
    Ok(match branch.strip_prefix("idea/") {
        Some(id) if is_valid_id(id) => ResString::Ok(s(id)),
        _ => ResString::Err(no_span(format!(
            "this worktree isn't an idea's: it has {branch} checked out"
        ))),
    })
}

/// The value of r, or d if it's an error.
pub fn ok_or(r: ResString, d: Str) -> Result<Str, String> {
    Ok(match r {
        ResString::Ok(x) => x,
        ResString::Err(_) => d,
    })
}

/// The first positional argument, or "".
pub fn arg0(a: CliArgs) -> Result<Str, String> {
    Ok(a.args.first().cloned().unwrap_or_default())
}

fn json_quote(x: &str) -> String {
    let mut o = String::from("\"");
    for c in x.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The command `grove run` types: its one argument as it is (a command line
/// for the shell), or its several each quoted for the shell, joined by
/// spaces.
pub fn run_cmd(a: CliArgs) -> Result<Str, String> {
    Ok(match a.args.len() {
        1 => a.args[0].clone(),
        _ => a
            .args
            .iter()
            .map(|x| sh_quote(x))
            .collect::<Vec<_>>()
            .join(" ")
            .into(),
    })
}

/// A run's JSON body: {"idea", "cmd", "new"}.
pub fn run_body(idea: Str, cmd: Str, fresh: bool) -> Result<Str, String> {
    Ok(format!(
        "{{\"idea\": {}, \"cmd\": {}, \"new\": {fresh}}}",
        json_quote(&idea),
        json_quote(&cmd)
    )
    .into())
}

/// The terminal of an idea's sections to run a command in: the first
/// terminal section's whose terminal is one of mine (opened in the idea's
/// worktree) and idle (its shell at its prompt); 0 if there's none.
pub fn run_terminal(xs: List<Section>, mine: List<i64>, idle: List<i64>) -> Result<i64, String> {
    Ok(xs
        .iter()
        .filter(|x| &*x.interior == "terminal" && x.term != 0)
        .map(|x| x.term)
        .find(|t| mine.contains(t) && idle.contains(t))
        .unwrap_or(0))
}

/// A notification's JSON body: {"idea", "open", "run"}.
pub fn notify_body(idea: Str, open: bool, run: Str) -> Result<Str, String> {
    Ok(format!(
        "{{\"idea\": {}, \"open\": {open}, \"run\": {}}}",
        json_quote(&idea),
        json_quote(&run)
    )
    .into())
}

/// Reads a bool field, or d if it's missing or not a bool.
pub fn bool_or(j: Json, key: Str, d: bool) -> Result<bool, String> {
    Ok(match field(&j, &key) {
        Some(Json::Bool(b)) => *b,
        _ => d,
    })
}

/// What's typed into a new terminal for run: run and a return, or nothing.
pub fn typed(run: Str) -> Result<Str, String> {
    Ok(if run.is_empty() {
        s("")
    } else {
        format!("{run}\r").into()
    })
}

/// An idea's sections with a terminal kept in them: its first terminal
/// section with none running gets it, expanded; else a new expanded terminal
/// section is added last.
pub fn with_terminal(xs: List<Section>, term: i64) -> Result<List<Section>, String> {
    let mut v: Vec<Section> = xs.iter().cloned().collect();
    match v
        .iter_mut()
        .find(|x| &*x.interior == "terminal" && x.term == 0)
    {
        Some(x) => {
            x.term = term;
            x.expanded = true;
        }
        None => v.push(Section {
            interior: s("terminal"),
            expanded: true,
            term,
        }),
    }
    Ok(v.into())
}

pub fn unwrap_i(r: ResInt) -> Result<i64, String> {
    match r {
        ResInt::Ok(n) => Ok(n),
        ResInt::Err(e) => Err(e.msg.to_string()),
    }
}

fn jfield(j: &Json, key: &str) -> String {
    match field(j, key) {
        Some(Json::Str(x)) => x.to_string(),
        _ => String::new(),
    }
}

fn tree_ideas(tree: &Json) -> Vec<&Json> {
    match field(tree, "ideas") {
        Some(Json::Arr(xs)) => xs.iter().collect(),
        _ => vec![],
    }
}

/// The tree (TreeJson) as text, an idea a line, indented under its parent,
/// with * on here's (none if here is "-").
pub fn tree_text(tree: Json, here: Str) -> Result<Str, String> {
    let ideas = tree_ideas(&tree);
    let mut kids: BTreeMap<String, Vec<&Json>> = BTreeMap::new();
    for i in ideas.iter().filter(|i| !jfield(i, "id").is_empty()) {
        kids.entry(jfield(i, "parent")).or_default().push(i);
    }
    let mut out = vec![];
    fn walk(
        i: &Json,
        depth: usize,
        here: &str,
        kids: &BTreeMap<String, Vec<&Json>>,
        out: &mut Vec<String>,
    ) {
        let id = jfield(i, "id");
        let mark = if id == here { "*" } else { " " };
        let shown = if id.is_empty() {
            "(root)".to_string()
        } else {
            id.clone()
        };
        let gone = if jfield(i, "status") == "discarded" {
            "  [discarded]"
        } else {
            ""
        };
        out.push(format!(
            "{mark} {}{shown:<10}  {}{gone}",
            "  ".repeat(depth),
            jfield(i, "name")
        ));
        for k in kids.get(&id).into_iter().flatten() {
            walk(k, depth + 1, here, kids, out);
        }
    }
    if let Some(root) = ideas.iter().find(|i| jfield(i, "id").is_empty()) {
        walk(root, 0, &here, &kids, &mut out);
    }
    Ok(out.join("\n").into())
}

/// An idea of the tree (TreeJson) as text: its id, name, branch, worktree
/// and parent; an error if the tree hasn't it.
pub fn idea_text(tree: Json, id: Str) -> Result<ResString, String> {
    let ideas = tree_ideas(&tree);
    let Some(i) = ideas.iter().find(|i| jfield(i, "id") == *id) else {
        return Ok(ResString::Err(no_span(format!(
            "idea {id} isn't in the tree: it has no branch idea/{id}"
        ))));
    };
    let parent = match field(i, "parent") {
        Some(Json::Str(p)) if p.is_empty() => "(root)".to_string(),
        Some(Json::Str(p)) => p.to_string(),
        _ => "none".to_string(),
    };
    Ok(ResString::Ok(
        format!(
            "id:       {}\nname:     {}\nbranch:   {}\nworktree: {}\nparent:   {parent}\nstatus:   {}",
            if id.is_empty() { "(root)" } else { &id },
            jfield(i, "name"),
            jfield(i, "branch"),
            jfield(i, "worktree"),
            jfield(i, "status")
        )
        .into(),
    ))
}

fn not_told(e: &Diag) -> String {
    format!(
        "grove wasn't told ({}); its tree catches up within a few seconds",
        e.msg
    )
}

/// What `grove branch` says it did: the new idea, where its worktree is, and
/// whether grove opened a terminal in it (notified: grove's answer, or why it
/// couldn't be told).
pub fn branched_text(
    id: Str,
    name: Str,
    from: Str,
    worktree: Str,
    open: bool,
    run: Str,
    notified: ResString,
) -> Result<Str, String> {
    let from = if from.is_empty() { s("the root") } else { from };
    let mut out = vec![format!(
        "branched idea {id}{} from {from}",
        if name.is_empty() {
            String::new()
        } else {
            format!(" ({name})")
        }
    )];
    out.push(format!("worktree: {worktree}"));
    match (&notified, open) {
        (ResString::Ok(_), true) if run.is_empty() => {
            out.push("grove opened a terminal in it".into())
        }
        (ResString::Ok(_), true) => {
            out.push(format!("grove opened a terminal in it, running: {run}"))
        }
        (ResString::Ok(_), false) => {}
        (ResString::Err(e), _) => {
            out.push(not_told(e));
            if open {
                out.push(if run.is_empty() {
                    "no terminal was opened in it".to_string()
                } else {
                    format!("no terminal was opened in it, so this didn't run: {run}")
                });
            }
        }
    }
    Ok(out.join("\n").into())
}

/// What a command says after a change, and whether grove was told:
/// done, then a note if notified is an error.
pub fn told_text(done: Str, notified: ResString) -> Result<Str, String> {
    Ok(match notified {
        ResString::Ok(_) => done,
        ResString::Err(e) => format!("{done}\n{}", not_told(&e)).into(),
    })
}

/// What `grove run` says it did, from grove's answer: where it ran cmd, and
/// whether in a new terminal; an error if grove couldn't be asked or
/// turned it away, which without $GROVE_URL says grove run needs grove
/// running.
pub fn ran_text(idea: Str, cmd: Str, answer: ResString) -> Result<ResString, String> {
    let at = if idea.is_empty() {
        "the root".to_string()
    } else {
        format!("idea {idea}")
    };
    Ok(match answer {
        // grove's answers are compact JSON.
        ResString::Ok(b) if b.contains("\"opened\":true") => {
            ResString::Ok(format!("grove opened a terminal in {at}, running: {cmd}").into())
        }
        ResString::Ok(_) => {
            ResString::Ok(format!("grove typed into a terminal of {at}: {cmd}").into())
        }
        ResString::Err(e) if e.msg.starts_with("no $GROVE_URL") => {
            ResString::Err(no_span(format!(
                "grove run types into grove's terminals, so it needs grove running: {}",
                e.msg
            )))
        }
        ResString::Err(e) => ResString::Err(no_span(format!("didn't run: {}", e.msg))),
    })
}

/// Prints nothing: what a command that succeeds quietly gives.
pub fn silent(_n: i64) -> Result<(), String> {
    Ok(())
}

/// A status's JSON body: {"idea", "state", "message"}, the state "" for
/// `clear`.
pub fn status_body(idea: Str, a: CliArgs) -> Result<Str, String> {
    let state = match a.args.first().map(|x| &**x) {
        Some("clear") | None => "",
        Some(x) => x,
    };
    let message = a
        .args
        .iter()
        .skip(1)
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(format!(
        "{{\"idea\": {}, \"state\": {}, \"message\": {}}}",
        json_quote(&idea),
        json_quote(state),
        json_quote(&message)
    )
    .into())
}

// ---- An idea's life (lifecycle.sys) ----

/// Ideas in the tree's order, parents before children, from the root: the
/// active ones only (one whose parent isn't among them hangs from the root).
pub fn tree_order(xs: List<Idea>) -> Result<List<Idea>, String> {
    let active: Vec<&Idea> = xs.iter().filter(|i| &*i.status == "active").collect();
    let ids: BTreeSet<&str> = active.iter().map(|i| &*i.id).collect();
    let parent = |i: &Idea| -> String {
        if ids.contains(&*i.parent) && i.parent != i.id {
            i.parent.to_string()
        } else {
            String::new()
        }
    };
    let mut out: Vec<Idea> = vec![];
    let mut level = vec![String::new()];
    let mut seen: BTreeSet<String> = BTreeSet::new();
    while !level.is_empty() {
        let mut next = vec![];
        for p in &level {
            let these: Vec<&Idea> = active
                .iter()
                .copied()
                .filter(|i| parent(i) == *p && !seen.contains(&*i.id))
                .collect();
            for i in these {
                seen.insert(i.id.to_string());
                out.push((*i).clone());
                next.push(i.id.to_string());
            }
        }
        level = next;
    }
    Ok(out.into())
}

/// An idea and every idea under it, in xs, children before their parents
/// (the idea itself last); none if xs hasn't it.
pub fn subtree(xs: List<Idea>, id: Str) -> Result<List<Idea>, String> {
    fn walk(xs: &List<Idea>, id: &str, seen: &mut BTreeSet<String>, out: &mut Vec<Idea>) {
        if !seen.insert(id.to_string()) {
            return;
        }
        for c in xs.iter().filter(|c| &*c.parent == id && &*c.id != id) {
            walk(xs, &c.id, seen, out);
        }
        if let Some(i) = xs.iter().find(|i| &*i.id == id) {
            out.push(i.clone());
        }
    }
    let mut out = vec![];
    if xs.iter().any(|i| i.id == id) {
        walk(&xs, &id, &mut BTreeSet::new(), &mut out);
    }
    Ok(out.into())
}

/// The ideas of xs whose parent is id.
pub fn children_of(xs: List<Idea>, id: Str) -> Result<List<Idea>, String> {
    Ok(xs
        .iter()
        .filter(|i| i.parent == id && i.id != id)
        .cloned()
        .collect())
}

/// Whether state is one an agent can report: working, attention, done, or
/// "" (none).
pub fn valid_state(state: Str) -> Result<bool, String> {
    Ok(matches!(&*state, "working" | "attention" | "done" | ""))
}

/// What a merge says it did: the idea merged, and the ideas that couldn't be
/// rebased onto the new root, each with why.
pub fn merged_text(name: Str, skipped: List<Str>) -> Result<Str, String> {
    Ok(if skipped.is_empty() {
        format!("merged {name} into the root; every other idea is on it now")
    } else {
        format!(
            "merged {name} into the root; not rebased onto it, so flagged: {}",
            skipped
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
    .into())
}

/// An idea's name, or its id if it has none.
pub fn name_of(i: Idea) -> Result<Str, String> {
    Ok(if i.name.is_empty() { i.id } else { i.name })
}

/// The error of r, or "" if it's Ok.
pub fn unwrap_err(r: ResString) -> Result<Str, String> {
    Ok(match r {
        ResString::Ok(_) => s(""),
        ResString::Err(e) => e.msg,
    })
}

pub fn unwrap_idea(r: ResIdea) -> Result<Idea, String> {
    match r {
        ResIdea::Ok(i) => Ok(i),
        ResIdea::Err(e) => Err(e.msg.to_string()),
    }
}

pub fn unwrap_ideas(r: ResIdeaList) -> Result<List<Idea>, String> {
    match r {
        ResIdeaList::Ok(i) => Ok(i),
        ResIdeaList::Err(e) => Err(e.msg.to_string()),
    }
}

pub fn unwrap_interiors(r: ResInteriorList) -> Result<List<Interior>, String> {
    match r {
        ResInteriorList::Ok(i) => Ok(i),
        ResInteriorList::Err(e) => Err(e.msg.to_string()),
    }
}

pub fn join_with(xs: List<Str>, sep: Str) -> Result<Str, String> {
    Ok(xs
        .iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(&sep)
        .into())
}

/// The idea i with another parent.
pub fn with_parent(i: Idea, parent: Str) -> Result<Idea, String> {
    Ok(Idea { parent, ..i })
}
