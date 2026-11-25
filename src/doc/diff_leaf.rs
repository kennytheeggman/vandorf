//! Leaves for the diff document (diff.sys): where a declaration is in its
//! text, which procs don't check, merging a proc's two layouts into one grid
//! with the changes marked, the changes to typedefs, interfaces and funcs,
//! and the summary `vcc diff` prints.

use super::doc_leaf::{edge_fields, field, is_unnamed_id, node_json, qual, show_ty, sig_of};
use crate::vcc::types::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

fn jstr(x: &str) -> Json {
    Json::Str(x.into())
}

fn obj(fs: Vec<(&str, Json)>) -> Json {
    Json::Obj(fs.into_iter().map(|(k, v)| field(k, v)).collect())
}

/// The field `key` of a JSON object, if it is one and has it.
fn get<'a>(j: &'a Json, key: &str) -> Option<&'a Json> {
    match j {
        Json::Obj(fs) => fs.iter().find(|f| &*f.key == key).map(|f| &f.value),
        _ => None,
    }
}

fn get_str<'a>(j: &'a Json, key: &str) -> &'a str {
    match get(j, key) {
        Some(Json::Str(s)) => s,
        _ => "",
    }
}

fn get_num(j: &Json, key: &str) -> i64 {
    match get(j, key) {
        Some(Json::Num(n)) => *n,
        _ => 0,
    }
}

fn fields_of(j: &Json) -> Vec<JsonField> {
    match j {
        Json::Obj(fs) => fs.to_vec(),
        _ => vec![],
    }
}

// ---- Declarations in their text ----

fn word_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
        i += 1;
    }
    i
}

/// Skips whitespace and `//` comments from i.
fn skip_ws(b: &[u8], mut i: usize) -> usize {
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if !b[i..].starts_with(b"//") {
            return i;
        }
        while i < b.len() && b[i] != b'\n' {
            i += 1;
        }
    }
}

/// Skips from the bracket `open` at i past the one that closes it, over
/// strings and `//` comments; to the end of the text if it isn't closed.
fn skip_balanced(b: &[u8], mut i: usize, open: u8, close: u8) -> usize {
    let mut depth = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

/// Skips a type as written from i: `[]T`, `map[K]V`, `{ .. }`, `enum { .. }`,
/// `Name` or `Name<..>`.
fn skip_type(b: &[u8], i: usize) -> usize {
    let i = skip_ws(b, i);
    match b.get(i) {
        Some(b'[') => skip_type(b, skip_balanced(b, i, b'[', b']')),
        Some(b'{') => skip_balanced(b, i, b'{', b'}'),
        Some(c) if c.is_ascii_alphabetic() || *c == b'_' => {
            let j = word_end(b, i);
            let k = skip_ws(b, j);
            match (&b[i..j], b.get(k)) {
                (b"enum", Some(b'{')) => skip_balanced(b, k, b'{', b'}'),
                (b"map", Some(b'[')) => skip_type(b, skip_balanced(b, k, b'[', b']')),
                (_, Some(b'<')) => skip_balanced(b, k, b'<', b'>'),
                _ => j,
            }
        }
        _ => i,
    }
}

/// Where the declaration whose name is at `at` starts and ends in src: from
/// the comment lines directly above its line to its closing `}` (a proc) or
/// its `;` (a func).
fn decl_extent(src: &str, at: usize) -> (usize, usize) {
    let mut at = at.min(src.len());
    while !src.is_char_boundary(at) {
        at -= 1;
    }
    let mut start = src[..at].rfind('\n').map_or(0, |i| i + 1);
    while start > 0 {
        let above = src[..start - 1].rfind('\n').map_or(0, |i| i + 1);
        if !src[above..start - 1].trim_start().starts_with("//") {
            break;
        }
        start = above;
    }
    let b = src.as_bytes();
    let mut i = skip_ws(b, word_end(b, at));
    if b.get(i) == Some(&b'(') {
        i = skip_ws(b, skip_balanced(b, i, b'(', b')'));
    }
    if b.get(i) == Some(&b':') {
        i = skip_ws(b, skip_type(b, i + 1));
    }
    let end = if b.get(i) == Some(&b'{') {
        skip_balanced(b, i, b'{', b'}')
    } else {
        // A func: up to its `;`, outside strings.
        let mut j = i;
        while j < b.len() && b[j] != b';' {
            if b[j] == b'"' {
                j += 1;
                while j < b.len() && b[j] != b'"' {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
            }
            j += 1;
        }
        (j + 1).min(b.len())
    };
    (start, end)
}

/// Finds the text of the declaration whose name is at `at` in src: from the
/// comment lines directly above it to its closing brace (a proc) or semicolon
/// (a func).
pub fn decl_text(src: Str, at: i64) -> Result<Str, String> {
    let (a, e) = decl_extent(&src, at.max(0) as usize);
    Ok(src[a..e].into())
}

/// A proc's declaration as written on a side.
fn proc_text(side: &DiffSide, pid: i64) -> Str {
    let span = &side.prog.nodes[pid as usize].span;
    match side.srcs.get(span.file as usize) {
        Some(src) => decl_text(src.clone(), span.at).unwrap_or_default(),
        None => Str::default(),
    }
}

// ---- Which procs don't check ----

/// Sorts errors by the top-level proc they're in, by name, each proc's
/// rendered with its text; every error under "" if one is outside every
/// top-level proc.
pub fn blame(
    file: File,
    srcs: List<Str>,
    paths: List<Str>,
    errors: List<Diag>,
) -> Result<Map<Str, FailedProc>, String> {
    let render =
        |es: Vec<Diag>| super::cli_leaf::render_errors(es.into(), paths.clone(), srcs.clone());
    // Each top-level proc: its name, file and extent.
    let procs: Vec<(Str, i64, usize, usize)> = file
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::Proc(p) => {
                let src = srcs.get(p.name.span.file as usize)?;
                let (a, e) = decl_extent(src, p.name.span.at as usize);
                Some((p.name.name.clone(), p.name.span.file, a, e))
            }
            Node::Func(_) => None,
        })
        .collect();
    let mut by: BTreeMap<Str, (Vec<Diag>, String)> = BTreeMap::new();
    for e in errors.iter() {
        let at = e.span.at.max(0) as usize;
        let Some((name, f, a, end)) = procs
            .iter()
            .find(|(_, f, a, end)| *f == e.span.file && *a <= at && at < *end)
        else {
            let all = FailedProc {
                error: render(errors.to_vec())?,
                text: Str::default(),
            };
            return Ok(Map::from_iter([(Str::default(), all)]));
        };
        let text = srcs[*f as usize][*a..*end].to_string();
        let entry = by.entry(name.clone()).or_insert_with(|| (vec![], text));
        entry.0.push(e.clone());
    }
    let mut out = BTreeMap::new();
    for (name, (es, text)) in by {
        out.insert(
            name,
            FailedProc {
                error: render(es)?,
                text: text.into(),
            },
        );
    }
    Ok(out.into())
}

/// Leaves the top-level procs named in `names` out of file.
pub fn without_procs(file: File, names: Map<Str, FailedProc>) -> Result<File, String> {
    let nodes = file
        .nodes
        .iter()
        .filter(|n| !matches!(n, Node::Proc(p) if names.contains_key(&p.name.name)))
        .cloned()
        .collect();
    Ok(File { nodes, ..file })
}

fn find_proc(p: &Program, name: &str) -> i64 {
    p.nodes
        .iter()
        .position(|n| matches!(n.kind, NodeKind::Proc(_)) && qual(n) == name)
        .map_or(-1, |i| i as i64)
}

/// Names every proc of either side, nested ones and ones that don't check
/// included, sorted.
pub fn proc_names(old: DiffSide, new: DiffSide) -> Result<List<Str>, String> {
    let mut names = BTreeSet::new();
    for side in [&old, &new] {
        for n in side.prog.nodes.iter() {
            if matches!(n.kind, NodeKind::Proc(_)) {
                names.insert(qual(n));
            }
        }
        names.extend(
            side.failed
                .keys()
                .filter(|k| !k.is_empty())
                .map(|k| k.to_string()),
        );
    }
    Ok(names.iter().map(|n| Str::from(n.as_str())).collect())
}

/// Finds the node of the proc named `name` in a side's program; -1 if it
/// has none.
pub fn proc_of(side: DiffSide, name: Str) -> Result<i64, String> {
    Ok(find_proc(&side.prog, &name))
}

fn proc_diff(status: &str, grid: Json, entry: Json) -> ProcDiff {
    ProcDiff {
        status: status.into(),
        grid,
        entry,
    }
}

/// Compares a proc that doesn't check on a side as text: its entry, with
/// each side's text and errors. Status compare if it checks on each side
/// it's on; skip if there's nothing to show (it doesn't check on either
/// side and its text didn't change, or it's inside a top-level proc whose
/// declaration doesn't check).
pub fn unchecked(old: DiffSide, new: DiffSide, name: Str) -> Result<ProcDiff, String> {
    let (of, nf) = (old.failed.get(&name), new.failed.get(&name));
    if of.is_none() && nf.is_none() {
        // A nested proc whose top-level proc was left out of a side.
        let covered = |side: &DiffSide| {
            find_proc(&side.prog, &name) < 0
                && name
                    .match_indices('.')
                    .any(|(i, _)| side.failed.contains_key(&name[..i]))
        };
        let status = if covered(&old) || covered(&new) {
            "skip"
        } else {
            "compare"
        };
        return Ok(proc_diff(status, Json::Null, Json::Null));
    }
    let text = |side: &DiffSide, f: Option<&FailedProc>| match f {
        Some(f) => Some(f.text.clone()),
        None => {
            let p = find_proc(&side.prog, &name);
            (p >= 0).then(|| proc_text(side, p))
        }
    };
    let (ot, nt) = (text(&old, of), text(&new, nf));
    if of.is_some() && nf.is_some() && ot == nt {
        return Ok(proc_diff("skip", Json::Null, Json::Null));
    }
    let mut errors = vec![];
    let mut texts = vec![];
    for (side, f, t) in [("old", of, ot), ("new", nf, nt)] {
        if let Some(f) = f {
            errors.push((side, Json::Str(f.error.clone())));
        }
        if let Some(t) = t {
            texts.push((side, Json::Str(t)));
        }
    }
    let entry = obj(vec![
        ("status", jstr("unchecked")),
        ("errors", obj(errors)),
        ("text", obj(texts)),
    ]);
    Ok(proc_diff("unchecked", Json::Null, entry))
}

// ---- Merging a proc's two layouts ----

fn unnamed(prog: &Program, n: i64) -> bool {
    let d = &prog.nodes[n as usize];
    matches!(d.kind, NodeKind::Id(_)) && d.qual.last().is_some_and(|l| is_unnamed_id(l))
}

/// One side's layout of a proc, with each placed node's key: its qualified
/// name, or for an id(..) without `as`, its source's key and `→id`.
struct Side<'a> {
    prog: &'a Program,
    pid: i64,
    l: &'a Layout,
    key: HashMap<i64, String>,
}

impl<'a> Side<'a> {
    fn new(d: &'a DiffSide, pid: i64, l: &'a Layout) -> Side<'a> {
        let prog = &d.prog;
        let mut key = HashMap::new();
        for &n in l.pos.keys() {
            if !unnamed(prog, n) {
                key.insert(n, qual(&prog.nodes[n as usize]));
            }
        }
        for (src, es) in l.out.iter() {
            for e in es.iter() {
                if l.pos.contains_key(&e.target) && unnamed(prog, e.target) {
                    let from = key.get(src).cloned().unwrap_or_default();
                    key.insert(e.target, format!("{from}→id"));
                }
            }
        }
        for &n in l.pos.keys() {
            key.entry(n)
                .or_insert_with(|| qual(&prog.nodes[n as usize]));
        }
        Side { prog, pid, l, key }
    }

    fn edges(&self, n: i64) -> Vec<Pending> {
        self.l.out.get(&n).map(|es| es.to_vec()).unwrap_or_default()
    }

    fn key_of(&self, n: i64) -> String {
        self.key
            .get(&n)
            .cloned()
            .unwrap_or_else(|| qual(&self.prog.nodes[n as usize]))
    }
}

/// Pairs the edges out of a node (ne, on the new side) with those out of its
/// match (oe, on the old): each new edge's old partner, if any. Edges pair
/// when their kinds and their targets' keys agree; those whose labels agree
/// too pair first, then the rest in order.
fn pair(n: &Side, ne: &[Pending], o: &Side, oe: &[Pending]) -> Vec<Option<usize>> {
    let group = |s: &Side, e: &Pending| (e.ty, s.key.get(&e.target).cloned().unwrap_or_default());
    let mut used = vec![false; oe.len()];
    let mut out = vec![None; ne.len()];
    for exact in [true, false] {
        for (i, e) in ne.iter().enumerate() {
            if out[i].is_some() {
                continue;
            }
            let g = group(n, e);
            let found = (0..oe.len())
                .find(|&j| !used[j] && group(o, &oe[j]) == g && (!exact || oe[j].label == e.label));
            if let Some(j) = found {
                out[i] = Some(j);
                used[j] = true;
            }
        }
    }
    out
}

/// The fields whose values differ between a node's or edge's new fields and
/// its old ones, each with its old value (null where it had none).
fn was(new: &[JsonField], old: &[JsonField]) -> Vec<JsonField> {
    let old_of = |k: &str| old.iter().find(|f| &*f.key == k).map(|f| f.value.clone());
    let mut out = vec![];
    for f in new {
        let o = old_of(&f.key);
        if o.as_ref() != Some(&f.value) {
            out.push(field(&f.key, o.unwrap_or(Json::Null)));
        }
    }
    for f in old {
        if !new.iter().any(|g| g.key == f.key) {
            out.push(field(&f.key, f.value.clone()));
        }
    }
    out
}

/// What was added, removed and changed among a proc's nodes, edges or
/// regions.
#[derive(Default, Clone, Copy)]
struct Tally {
    added: usize,
    removed: usize,
    changed: usize,
}

impl Tally {
    fn json(&self) -> Json {
        obj(vec![
            ("added", Json::Num(self.added as i64)),
            ("removed", Json::Num(self.removed as i64)),
            ("changed", Json::Num(self.changed as i64)),
        ])
    }
    fn any(&self) -> bool {
        self.added + self.removed + self.changed > 0
    }
}

/// A for each region as the diff compares it: the node that opens it, the
/// nodes in it and the one that collects it, by key, and its element type.
struct Bounds {
    opener: i64,
    key: String,
    nodes: Vec<String>,
    sink: String,
    elem: String,
}

impl Bounds {
    fn fields(&self) -> Vec<JsonField> {
        vec![
            field(
                "nodes",
                Json::Arr(self.nodes.iter().map(|n| jstr(n)).collect()),
            ),
            field("sink", jstr(&self.sink)),
            field("elem", jstr(&self.elem)),
        ]
    }
}

fn regions(side: &Side) -> Vec<Bounds> {
    if side.pid < 0 {
        return vec![];
    }
    let NodeKind::Proc(p) = &side.prog.nodes[side.pid as usize].kind else {
        return vec![];
    };
    let g = &p.graph;
    let mut out: Vec<Bounds> = g
        .opens
        .iter()
        .filter_map(|(&opener, &r)| {
            let reg = g.regions.get(r as usize)?;
            let mut nodes: Vec<String> = reg.nodes.iter().map(|&x| side.key_of(x)).collect();
            nodes.sort();
            Some(Bounds {
                opener,
                key: side.key_of(opener),
                nodes,
                sink: if reg.sink >= 0 {
                    side.key_of(reg.sink)
                } else {
                    String::new()
                },
                elem: show_ty(&reg.elem),
            })
        })
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

/// A cell of the merged grid: a node of the new layout, or a removed node of
/// the old.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Cell {
    New(i64),
    Old(i64),
}

fn find(rows: &[Vec<Cell>], c: Cell) -> Option<(usize, usize)> {
    rows.iter()
        .enumerate()
        .find_map(|(r, row)| row.iter().position(|&x| x == c).map(|i| (r, i)))
}

/// Merges a proc's two layouts into one grid: the new one, with removed
/// nodes put back beside the neighbours they had, and each node, edge and
/// for each region that was added, removed or changed marked; and its
/// entry. A pid of -1: that side doesn't have the proc.
pub fn merge(
    old: DiffSide,
    opid: i64,
    ol: Layout,
    new: DiffSide,
    npid: i64,
    nl: Layout,
) -> Result<ProcDiff, String> {
    let o = Side::new(&old, opid, &ol);
    let n = Side::new(&new, npid, &nl);

    // Nodes match by key; an id(..) without `as` goes with the edge into it.
    let old_keys: HashMap<&str, i64> = o
        .key
        .iter()
        .filter(|&(&m, _)| !unnamed(o.prog, m))
        .map(|(&m, k)| (k.as_str(), m))
        .collect();
    let mut n2o: HashMap<i64, i64> = HashMap::new();
    for (&x, k) in &n.key {
        if !unnamed(n.prog, x)
            && let Some(&m) = old_keys.get(k.as_str())
        {
            n2o.insert(x, m);
        }
    }
    let mut pairs: HashMap<i64, Vec<Option<usize>>> = HashMap::new();
    for (x, m) in n2o.clone() {
        let (ne, oe) = (n.edges(x), o.edges(m));
        let p = pair(&n, &ne, &o, &oe);
        for (i, j) in p.iter().enumerate() {
            if let Some(j) = j
                && unnamed(n.prog, ne[i].target)
                && unnamed(o.prog, oe[*j].target)
            {
                n2o.insert(ne[i].target, oe[*j].target);
            }
        }
        pairs.insert(x, p);
    }
    let o2n: HashMap<i64, i64> = n2o.iter().map(|(&a, &b)| (b, a)).collect();

    // The new layout, with each removed node put back beside the nearest
    // neighbour it had in its row (after one on its left, or before one on its
    // right); failing that, in the row below the nodes leading to it, or above
    // those it leads to, or in its old row.
    let mut rows: Vec<Vec<Cell>> = nl
        .rows
        .iter()
        .map(|r| r.iter().map(|&x| Cell::New(x)).collect())
        .collect();
    let locate = |rows: &[Vec<Cell>], m: i64| match o2n.get(&m) {
        Some(&x) => find(rows, Cell::New(x)),
        None => find(rows, Cell::Old(m)),
    };
    for (r, row) in ol.rows.iter().enumerate() {
        for (c, &m) in row.iter().enumerate() {
            if o2n.contains_key(&m) {
                continue;
            }
            let beside = (0..c)
                .rev()
                .find_map(|k| locate(&rows, row[k]).map(|(rr, cc)| (rr, cc + 1)))
                .or_else(|| (c + 1..row.len()).find_map(|k| locate(&rows, row[k])));
            if let Some((rr, cc)) = beside {
                rows[rr].insert(cc, Cell::Old(m));
                continue;
            }
            let preds = ol
                .out
                .iter()
                .filter(|(_, es)| es.iter().any(|e| e.target == m && !e.back))
                .filter_map(|(&src, _)| locate(&rows, src))
                .map(|p| p.0)
                .max();
            let succs = o
                .edges(m)
                .iter()
                .filter(|e| !e.back)
                .filter_map(|e| locate(&rows, e.target))
                .map(|p| p.0)
                .min();
            let at = match (preds, succs) {
                (Some(p), _) => p + 1,
                (None, Some(s)) => s.saturating_sub(1),
                (None, None) => r.min(rows.len()),
            };
            if at >= rows.len() {
                rows.push(vec![]);
            }
            let at = at.min(rows.len() - 1);
            rows[at].push(Cell::Old(m));
        }
    }
    let pos: HashMap<Cell, (i64, i64)> = rows
        .iter()
        .enumerate()
        .flat_map(|(r, row)| {
            row.iter()
                .enumerate()
                .map(move |(c, &x)| (x, (r as i64, c as i64)))
        })
        .collect();
    let old_cell = |m: i64| o2n.get(&m).map_or(Cell::Old(m), |&x| Cell::New(x));

    // Nodes.
    let (mut nodes, mut edge_tally, mut region_tally) =
        (Tally::default(), Tally::default(), Tally::default());
    let tag = |fs: &mut Vec<JsonField>, diff: &str| fs.push(field("diff", jstr(diff)));
    let mut row_json = vec![];
    for row in &rows {
        let mut cells = vec![];
        for &cell in row {
            let fs = match cell {
                Cell::New(x) => {
                    let mut fs = fields_of(&node_json(n.prog, n.pid, x));
                    match n2o.get(&x) {
                        Some(&m) => {
                            let w = was(&fs, &fields_of(&node_json(o.prog, o.pid, m)));
                            if !w.is_empty() {
                                tag(&mut fs, "changed");
                                fs.push(field("was", Json::Obj(w.into())));
                                nodes.changed += 1;
                            }
                        }
                        None => {
                            tag(&mut fs, "added");
                            nodes.added += 1;
                        }
                    }
                    fs
                }
                Cell::Old(m) => {
                    let mut fs = fields_of(&node_json(o.prog, o.pid, m));
                    tag(&mut fs, "removed");
                    nodes.removed += 1;
                    fs
                }
            };
            cells.push(Json::Obj(fs.into()));
        }
        row_json.push(obj(vec![("nodes", Json::Arr(cells.into()))]));
    }

    // Edges, from each cell in turn: the new edges, then the removed ones.
    let mut edges = vec![];
    let removed = |edges: &mut Vec<Json>, from: (i64, i64), e: &Pending, tally: &mut Tally| {
        if let Some(&t) = pos.get(&old_cell(e.target)) {
            let mut fs = edge_fields(e, from, t);
            fs.push(field("diff", jstr("removed")));
            edges.push(Json::Obj(fs.into()));
            tally.removed += 1;
        }
    };
    for row in &rows {
        for &cell in row {
            let from = pos[&cell];
            match cell {
                Cell::New(x) => {
                    let ne = n.edges(x);
                    let (oe, p) = match n2o.get(&x) {
                        Some(&m) => (o.edges(m), pairs.get(&x).cloned().unwrap_or_default()),
                        None => (vec![], vec![]),
                    };
                    for (i, e) in ne.iter().enumerate() {
                        let Some(&t) = pos.get(&Cell::New(e.target)) else {
                            continue;
                        };
                        let mut fs = edge_fields(e, from, t);
                        match p.get(i).copied().flatten() {
                            Some(j) => {
                                let w = was(&edge_parts(e), &edge_parts(&oe[j]));
                                if !w.is_empty() {
                                    fs.push(field("diff", jstr("changed")));
                                    fs.push(field("was", Json::Obj(w.into())));
                                    edge_tally.changed += 1;
                                }
                            }
                            None => {
                                fs.push(field("diff", jstr("added")));
                                edge_tally.added += 1;
                            }
                        }
                        edges.push(Json::Obj(fs.into()));
                    }
                    let paired: HashSet<usize> = p.iter().flatten().copied().collect();
                    for (j, e) in oe.iter().enumerate() {
                        if !paired.contains(&j) {
                            removed(&mut edges, from, e, &mut edge_tally);
                        }
                    }
                }
                Cell::Old(m) => {
                    for e in o.edges(m) {
                        removed(&mut edges, from, &e, &mut edge_tally);
                    }
                }
            }
        }
    }

    // For each regions, by the node that opens them.
    let (orr, nrr) = (regions(&o), regions(&n));
    let mut region_json = vec![];
    for b in &nrr {
        let at = pos.get(&Cell::New(b.opener)).copied().unwrap_or((0, 0));
        let mut fs = vec![field("opener", jstr(&b.key))];
        fs.push(field(
            "at",
            Json::Arr(vec![Json::Num(at.0), Json::Num(at.1)].into()),
        ));
        match orr.iter().find(|a| a.key == b.key) {
            Some(a) => {
                let w = was(&b.fields(), &a.fields());
                if w.is_empty() {
                    continue;
                }
                fs.push(field("diff", jstr("changed")));
                fs.extend(b.fields());
                fs.push(field("was", Json::Obj(w.into())));
                region_tally.changed += 1;
            }
            None => {
                fs.push(field("diff", jstr("added")));
                fs.extend(b.fields());
                region_tally.added += 1;
            }
        }
        region_json.push(Json::Obj(fs.into()));
    }
    for a in orr.iter().filter(|a| !nrr.iter().any(|b| b.key == a.key)) {
        let at = pos.get(&old_cell(a.opener)).copied().unwrap_or((0, 0));
        let mut fs = vec![field("opener", jstr(&a.key))];
        fs.push(field(
            "at",
            Json::Arr(vec![Json::Num(at.0), Json::Num(at.1)].into()),
        ));
        fs.push(field("diff", jstr("removed")));
        fs.extend(a.fields());
        region_json.push(Json::Obj(fs.into()));
        region_tally.removed += 1;
    }

    let status = if opid < 0 {
        "added"
    } else if npid < 0 {
        "removed"
    } else if nodes.any() || edge_tally.any() || region_tally.any() {
        "changed"
    } else {
        "unchanged"
    };
    let mut texts = vec![];
    if opid >= 0 {
        texts.push(("old", Json::Str(proc_text(&old, opid))));
    }
    if npid >= 0 {
        texts.push(("new", Json::Str(proc_text(&new, npid))));
    }
    let entry = obj(vec![
        ("status", jstr(status)),
        (
            "counts",
            obj(vec![
                ("nodes", nodes.json()),
                ("edges", edge_tally.json()),
                ("regions", region_tally.json()),
            ]),
        ),
        ("regions", Json::Arr(region_json.into())),
        ("text", obj(texts)),
    ]);
    let grid = obj(vec![
        ("rows", Json::Arr(row_json.into())),
        ("edges", Json::Arr(edges.into())),
    ]);
    Ok(proc_diff(status, grid, entry))
}

/// What an edge's change is judged by: its label (a switch's arm heads) and
/// its arguments. Its value type and call follow from its nodes.
fn edge_parts(e: &Pending) -> Vec<JsonField> {
    let mut fs = vec![];
    if let MaybeStr::Some(l) = &e.label {
        fs.push(field("label", Json::Str(l.clone())));
    }
    if !e.args.is_empty() {
        fs.push(field("args", Json::Str(e.args.clone())));
    }
    fs
}

// ---- Typedefs, interfaces and funcs ----

/// A func's signature, implementations and doc, as the diff compares them.
fn func_fields(p: &Program, id: i64) -> Vec<JsonField> {
    let d = &p.nodes[id as usize];
    let mut fs = vec![];
    if let Some(sig) = sig_of(p, id) {
        fs.push(field("sig", jstr(&sig)));
    }
    if let NodeKind::Func(f) = &d.kind
        && !f.imps.is_empty()
    {
        let imps: Vec<String> = f
            .imps
            .iter()
            .map(|i| format!("{}:{}", i.file, i.function))
            .collect();
        fs.push(field("impl", jstr(&imps.join(" · "))));
    }
    if !d.doc.is_empty() {
        fs.push(field("doc", Json::Str(d.doc.clone())));
    }
    fs
}

/// Compares two sets of named things by their fields, in name order: an
/// entry for each one added or removed (its fields) or changed (its new
/// fields, and under `was` the old values of those that differ).
fn by_name(
    old: &BTreeMap<String, Vec<JsonField>>,
    new: &BTreeMap<String, Vec<JsonField>>,
) -> Vec<(String, Json)> {
    let names: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    let mut out = vec![];
    for name in names {
        let (status, mut fs) = match (old.get(name), new.get(name)) {
            (None, Some(b)) => ("added", b.clone()),
            (Some(a), None) => ("removed", a.clone()),
            (Some(a), Some(b)) => {
                let w = was(b, a);
                if w.is_empty() {
                    continue;
                }
                let mut fs = b.clone();
                fs.push(field("was", Json::Obj(w.into())));
                ("changed", fs)
            }
            (None, None) => continue,
        };
        fs.insert(0, field("status", jstr(status)));
        out.push((name.clone(), Json::Obj(fs.into())));
    }
    out
}

/// A struct's fields or an enum's variants that were added, removed or
/// changed, in the new order, then the removed ones.
fn members(old: &[Member], new: &[Member]) -> Json {
    let shown = |m: &Member| show_ty(&m.ty);
    let entry = |m: &Member, status: &str| {
        vec![
            field("name", Json::Str(m.name.clone())),
            field("status", jstr(status)),
            field("type", jstr(&shown(m))),
        ]
    };
    let mut out = vec![];
    for m in new {
        match old.iter().find(|o| o.name == m.name) {
            None => out.push(Json::Obj(entry(m, "added").into())),
            Some(o) if shown(o) != shown(m) => {
                let mut fs = entry(m, "changed");
                fs.push(field("was", jstr(&shown(o))));
                out.push(Json::Obj(fs.into()));
            }
            Some(_) => {}
        }
    }
    for o in old.iter().filter(|o| !new.iter().any(|m| m.name == o.name)) {
        out.push(Json::Obj(entry(o, "removed").into()));
    }
    Json::Arr(out.into())
}

fn types_json(old: &Program, new: &Program) -> Json {
    let defs = |p: &Program| -> BTreeMap<String, TypeDef> {
        p.types
            .iter()
            .map(|t| (t.name.to_string(), t.clone()))
            .collect()
    };
    let fields = |t: &TypeDef| {
        let mut fs = vec![field("def", jstr(&show_ty(&t.ty)))];
        if !t.doc.is_empty() {
            fs.push(field("doc", Json::Str(t.doc.clone())));
        }
        fs
    };
    let (od, nd) = (defs(old), defs(new));
    let of: BTreeMap<String, Vec<JsonField>> =
        od.iter().map(|(k, t)| (k.clone(), fields(t))).collect();
    let nf: BTreeMap<String, Vec<JsonField>> =
        nd.iter().map(|(k, t)| (k.clone(), fields(t))).collect();
    // An instance of a generic typedef comes and goes with its uses; it's
    // listed when it changes, as its generic's body did.
    let use_only =
        |name: &str| name.contains('<') && !(od.contains_key(name) && nd.contains_key(name));
    let out = by_name(&of, &nf)
        .into_iter()
        .filter(|(name, _)| !use_only(name))
        .map(|(name, j)| {
            let mut fs = fields_of(&j);
            if let (Some(a), Some(b)) = (od.get(&name), nd.get(&name)) {
                match (&a.ty, &b.ty) {
                    (Type::Struct(x), Type::Struct(y)) => fs.push(field("fields", members(x, y))),
                    (Type::Enum(x), Type::Enum(y)) => fs.push(field("variants", members(x, y))),
                    _ => {}
                }
            }
            field(&name, Json::Obj(fs.into()))
        })
        .collect();
    Json::Obj(out)
}

fn interfaces_json(old: &Program, new: &Program) -> Json {
    let ifaces = |p: &Program| -> BTreeMap<String, BTreeMap<String, Vec<JsonField>>> {
        p.globals
            .iter()
            .filter_map(|(k, g)| match g {
                Global::Interface(fs) => Some((
                    k.to_string(),
                    fs.iter()
                        .map(|(f, &id)| (f.to_string(), func_fields(p, id)))
                        .collect(),
                )),
                Global::Node(_) => None,
            })
            .collect()
    };
    let (oi, ni) = (ifaces(old), ifaces(new));
    let names: BTreeSet<&String> = oi.keys().chain(ni.keys()).collect();
    let empty = BTreeMap::new();
    let mut out = vec![];
    for name in names {
        let (a, b) = (oi.get(name), ni.get(name));
        let funcs = by_name(a.unwrap_or(&empty), b.unwrap_or(&empty));
        let status = match (a, b) {
            (None, _) => "added",
            (_, None) => "removed",
            _ if funcs.is_empty() => continue,
            _ => "changed",
        };
        let funcs: Vec<Json> = funcs
            .into_iter()
            .map(|(f, j)| {
                let mut fs = vec![field("name", jstr(&f))];
                fs.extend(fields_of(&j));
                Json::Obj(fs.into())
            })
            .collect();
        out.push(field(
            name,
            obj(vec![
                ("status", jstr(status)),
                ("funcs", Json::Arr(funcs.into())),
            ]),
        ));
    }
    Json::Obj(out.into())
}

fn funcs_json(old: &Program, new: &Program) -> Json {
    let funcs = |p: &Program| -> BTreeMap<String, Vec<JsonField>> {
        p.globals
            .iter()
            .filter_map(|(k, g)| match g {
                Global::Node(id) if matches!(p.nodes[*id as usize].kind, NodeKind::Func(_)) => {
                    Some((k.to_string(), func_fields(p, *id)))
                }
                _ => None,
            })
            .collect()
    };
    let out: Vec<JsonField> = by_name(&funcs(old), &funcs(new))
        .into_iter()
        .map(|(k, j)| field(&k, j))
        .collect();
    Json::Obj(out.into())
}

/// Compares the typedefs, interfaces and top-level funcs of two sides that
/// check; none if one doesn't.
pub fn decl_changes(old: DiffSide, new: DiffSide) -> Result<DeclChanges, String> {
    if !old.checks || !new.checks {
        return Ok(DeclChanges {
            types: obj(vec![]),
            interfaces: obj(vec![]),
            funcs: obj(vec![]),
        });
    }
    Ok(DeclChanges {
        types: types_json(&old.prog, &new.prog),
        interfaces: interfaces_json(&old.prog, &new.prog),
        funcs: funcs_json(&old.prog, &new.prog),
    })
}

// ---- The rest of the document ----

/// Describes a side: its label, whether it checks, and if not why.
pub fn side_json(s: DiffSide) -> Result<Json, String> {
    let mut fs = vec![
        ("label", Json::Str(s.label)),
        ("checks", Json::Bool(s.checks)),
    ];
    if !s.checks {
        fs.push(("error", Json::Str(s.error)));
    }
    Ok(obj(fs))
}

/// Lists the .sys files added, removed or changed between two sides, by
/// path.
pub fn files_json(old: DiffSide, new: DiffSide) -> Result<Json, String> {
    let files = |s: &DiffSide| -> BTreeMap<Str, Str> {
        s.paths
            .iter()
            .cloned()
            .zip(s.srcs.iter().cloned())
            .collect()
    };
    let (of, nf) = (files(&old), files(&new));
    let paths: BTreeSet<&Str> = of.keys().chain(nf.keys()).collect();
    let mut out = vec![];
    for p in paths {
        let status = match (of.get(p), nf.get(p)) {
            (None, _) => "added",
            (_, None) => "removed",
            (Some(a), Some(b)) if a != b => "changed",
            _ => continue,
        };
        out.push(obj(vec![
            ("path", Json::Str(p.clone())),
            ("status", jstr(status)),
        ]));
    }
    Ok(Json::Arr(out.into()))
}

fn tally(entries: &[&Json], statuses: &[&str]) -> Json {
    obj(statuses
        .iter()
        .map(|s| {
            let n = entries
                .iter()
                .filter(|e| get_str(e, "status") == *s)
                .count();
            (*s, Json::Num(n as i64))
        })
        .collect())
}

fn entries(j: &Json) -> Vec<&Json> {
    match j {
        Json::Obj(fs) => fs.iter().map(|f| &f.value).collect(),
        _ => vec![],
    }
}

/// Counts what was added, removed and changed (and procs that don't check)
/// among the procs' entries and the other declarations.
pub fn counts_json(procs: List<JsonField>, decls: DeclChanges) -> Result<Json, String> {
    let three = ["added", "removed", "changed"];
    let procs: Vec<&Json> = procs.iter().map(|f| &f.value).collect();
    Ok(obj(vec![
        (
            "procs",
            tally(&procs, &["added", "removed", "changed", "unchecked"]),
        ),
        ("types", tally(&entries(&decls.types), &three)),
        ("interfaces", tally(&entries(&decls.interfaces), &three)),
        ("funcs", tally(&entries(&decls.funcs), &three)),
    ]))
}

// ---- The summary ----

fn mark(status: &str) -> &'static str {
    match status {
        "added" => "+",
        "removed" => "-",
        "unchecked" => "!",
        _ => "~",
    }
}

/// `+1 -2 ~3`, leaving out what's zero.
fn plus_minus(t: Option<&Json>) -> String {
    let Some(t) = t else { return String::new() };
    let parts: Vec<String> = [("+", "added"), ("-", "removed"), ("~", "changed")]
        .iter()
        .filter(|(_, k)| get_num(t, k) > 0)
        .map(|(m, k)| format!("{m}{}", get_num(t, k)))
        .collect();
    parts.join(" ")
}

/// A section's heading: `procs: 1 changed, 2 added`, leaving out what's
/// zero; None if all are.
fn heading(what: &str, counts: Option<&Json>) -> Option<String> {
    let counts = counts?;
    let parts: Vec<String> = fields_of(counts)
        .iter()
        .filter_map(|f| match f.value {
            Json::Num(n) if n > 0 => Some(match &*f.key {
                "unchecked" => format!("{n} not checking"),
                k => format!("{n} {k}"),
            }),
            _ => None,
        })
        .collect();
    (!parts.is_empty()).then(|| format!("{what}: {}", parts.join(", ")))
}

/// A line of a section: its mark, name and detail, and lines to go under it.
type Item = (String, String, String, Vec<String>);

/// Lines `  ~ name  detail`, names padded to one width, each followed by the
/// lines under it.
fn item_lines(items: Vec<Item>, out: &mut Vec<String>) {
    let w = items.iter().map(|i| i.1.chars().count()).max().unwrap_or(0);
    for (m, name, detail, under) in items {
        let line = format!("  {m} {name:<w$}  {detail}");
        out.push(line.trim_end().to_string());
        out.extend(under);
    }
}

fn indent(text: &str, by: &str) -> String {
    text.lines()
        .map(|l| format!("{by}{l}").trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Writes a diff document's summary.
pub fn summary(doc: Json) -> Result<Str, String> {
    let (old, new) = (get(&doc, "old"), get(&doc, "new"));
    let label = |s: Option<&Json>| s.map_or("", |s| get_str(s, "label")).to_string();
    let checks =
        |s: Option<&Json>| matches!(s.and_then(|s| get(s, "checks")), Some(Json::Bool(true)));
    let mut out = vec![format!("{} → {}", label(old), label(new))];
    for side in [old, new] {
        if !checks(side) {
            out.push(format!(
                "{} doesn't check, so its procs can't be compared; compare the files' text instead:",
                label(side)
            ));
            out.push(indent(side.map_or("", |s| get_str(s, "error")), "    "));
        }
    }
    let counts = get(&doc, "counts");
    let section = |what: &str| counts.and_then(|c| get(c, what));
    let mut any = false;

    // Procs: what changed in each.
    if let Some(h) = heading("procs", section("procs")) {
        any = true;
        out.push(h);
        let mut items = vec![];
        for f in get(&doc, "procs").map(fields_of).unwrap_or_default() {
            let st = get_str(&f.value, "status");
            let mut under = vec![];
            let detail = if st == "unchecked" {
                let errs = get(&f.value, "errors").map(fields_of).unwrap_or_default();
                let sides: Vec<String> = errs
                    .iter()
                    .map(|e| label(if &*e.key == "old" { old } else { new }))
                    .collect();
                for e in &errs {
                    if let Json::Str(s) = &e.value {
                        under.push(indent(s, "      "));
                    }
                }
                format!("doesn't check in {}; compare its text", sides.join(" or "))
            } else {
                let c = get(&f.value, "counts");
                let parts: Vec<String> = ["nodes", "edges", "regions"]
                    .iter()
                    .filter_map(|k| {
                        let t = plus_minus(c.and_then(|c| get(c, k)));
                        (!t.is_empty()).then(|| format!("{k} {t}"))
                    })
                    .collect();
                parts.join(", ")
            };
            items.push((mark(st).to_string(), f.key.to_string(), detail, under));
        }
        item_lines(items, &mut out);
    }

    // Typedefs: their fields or variants.
    if let Some(h) = heading("types", section("types")) {
        any = true;
        out.push(h);
        let mut items = vec![];
        for f in get(&doc, "types").map(fields_of).unwrap_or_default() {
            let st = get_str(&f.value, "status");
            let mut parts = vec![];
            for k in ["fields", "variants"] {
                if let Some(Json::Arr(ms)) = get(&f.value, k) {
                    let n = |s: &str| ms.iter().filter(|m| get_str(m, "status") == s).count();
                    let t = plus_minus(Some(&obj(vec![
                        ("added", Json::Num(n("added") as i64)),
                        ("removed", Json::Num(n("removed") as i64)),
                        ("changed", Json::Num(n("changed") as i64)),
                    ])));
                    if !t.is_empty() {
                        parts.push(format!("{k} {t}"));
                    }
                }
            }
            let was = get(&f.value, "was");
            if parts.is_empty()
                && let Some(Json::Str(d)) = was.and_then(|w| get(w, "def"))
            {
                parts.push(format!("{d} → {}", get_str(&f.value, "def")));
            }
            if was.and_then(|w| get(w, "doc")).is_some() {
                parts.push("doc".to_string());
            }
            let item = (
                mark(st).to_string(),
                f.key.to_string(),
                parts.join(", "),
                vec![],
            );
            items.push(item);
        }
        item_lines(items, &mut out);
    }

    // Interfaces: their funcs.
    if let Some(h) = heading("interfaces", section("interfaces")) {
        any = true;
        out.push(h);
        let mut items = vec![];
        for f in get(&doc, "interfaces").map(fields_of).unwrap_or_default() {
            let st = get_str(&f.value, "status");
            let fs = match get(&f.value, "funcs") {
                Some(Json::Arr(fs)) => fs.to_vec(),
                _ => vec![],
            };
            let n = |s: &str| fs.iter().filter(|x| get_str(x, "status") == s).count();
            let t = plus_minus(Some(&obj(vec![
                ("added", Json::Num(n("added") as i64)),
                ("removed", Json::Num(n("removed") as i64)),
                ("changed", Json::Num(n("changed") as i64)),
            ])));
            items.push((
                mark(st).to_string(),
                f.key.to_string(),
                format!("funcs {t}"),
                vec![],
            ));
        }
        item_lines(items, &mut out);
    }

    // Top-level funcs: what about them changed.
    if let Some(h) = heading("funcs", section("funcs")) {
        any = true;
        out.push(h);
        let mut items = vec![];
        for f in get(&doc, "funcs").map(fields_of).unwrap_or_default() {
            let st = get_str(&f.value, "status");
            let w = get(&f.value, "was").map(fields_of).unwrap_or_default();
            let detail = w
                .iter()
                .map(|w| w.key.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            items.push((mark(st).to_string(), f.key.to_string(), detail, vec![]));
        }
        item_lines(items, &mut out);
    }

    // Files.
    let files = match get(&doc, "files") {
        Some(Json::Arr(fs)) => fs.to_vec(),
        _ => vec![],
    };
    if !files.is_empty() {
        let n = |s: &str| files.iter().filter(|f| get_str(f, "status") == s).count();
        let counts = obj(vec![
            ("changed", Json::Num(n("changed") as i64)),
            ("added", Json::Num(n("added") as i64)),
            ("removed", Json::Num(n("removed") as i64)),
        ]);
        if let Some(h) = heading("files", Some(&counts)) {
            out.push(h);
        }
        let items = files
            .iter()
            .map(|f| {
                (
                    mark(get_str(f, "status")).to_string(),
                    get_str(f, "path").to_string(),
                    String::new(),
                    vec![],
                )
            })
            .collect();
        item_lines(items, &mut out);
    }
    if !any && checks(old) && checks(new) {
        out.push(if files.is_empty() {
            "no changes".to_string()
        } else {
            "no changes to procs, types, interfaces or funcs".to_string()
        });
    }
    Ok(out.join("\n").into())
}
