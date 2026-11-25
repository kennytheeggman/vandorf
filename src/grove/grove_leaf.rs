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
/// `git worktree list --porcelain`; "" if none has.
pub fn worktree_of(porcelain: Str, id: Str) -> Result<Str, String> {
    let want = format!("refs/heads/idea/{id}");
    Ok(worktrees(&porcelain)
        .into_iter()
        .find(|w| w.branch == want)
        .map_or(s(""), |w| w.path.into()))
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
pub fn tree_json(
    repo: Repo,
    worktrees: Str,
    branches: Str,
    root: Idea,
    ideas: List<Idea>,
    interiors: List<Interior>,
) -> Result<Json, String> {
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
    ])];
    for i in ideas.iter().filter(|i| known.contains(&*i.id)) {
        let branch = format!("refs/heads/idea/{}", i.id);
        let wt = wts.iter().find(|w| w.branch == branch);
        let parent = if known.contains(&*i.parent) && i.parent != i.id {
            &*i.parent
        } else {
            ""
        };
        out.push(obj(vec![
            ("id", Json::Str(i.id.clone())),
            ("name", Json::Str(i.name.clone())),
            ("parent", jstr(parent)),
            ("fork", Json::Str(i.fork.clone())),
            (
                "status",
                jstr(if wt.is_some() { "active" } else { "discarded" }),
            ),
            ("branch", jstr(&branch["refs/heads/".len()..])),
            ("head", jstr(heads.get(&*i.id).copied().unwrap_or(""))),
            ("worktree", jstr(wt.map_or("", |w| &w.path))),
            ("sections", sections_json(&i.sections)),
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
