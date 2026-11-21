//! Leaves for vcc's document layout (../sys/doc): grid updates and JSON.

use crate::vcc::types::*;

fn err<T>(what: &str) -> Result<T, String> {
    Err(format!("doc leaf: {what}"))
}

// ---- JSON, as serde_json::to_string_pretty writes it ----

fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write(j: &Json, indent: usize, out: &mut String) {
    let pad = |n: usize| "  ".repeat(n);
    match j {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Num(i) => out.push_str(&i.to_string()),
        Json::Str(s) => escape(s, out),
        Json::Arr(xs) if xs.is_empty() => out.push_str("[]"),
        Json::Arr(xs) => {
            out.push_str("[\n");
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&pad(indent + 1));
                write(x, indent + 1, out);
            }
            out.push('\n');
            out.push_str(&pad(indent));
            out.push(']');
        }
        Json::Obj(fs) if fs.is_empty() => out.push_str("{}"),
        Json::Obj(fs) => {
            out.push_str("{\n");
            for (i, f) in fs.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&pad(indent + 1));
                escape(&f.key, out);
                out.push_str(": ");
                write(&f.value, indent + 1, out);
            }
            out.push('\n');
            out.push_str(&pad(indent));
            out.push('}');
        }
    }
}

pub fn pretty(j: Json) -> Result<Str, String> {
    let mut out = String::new();
    write(&j, 0, &mut out);
    Ok(out.into())
}

pub fn fields(m: Map<Str, Json>) -> Result<List<JsonField>, String> {
    Ok(m.iter().map(|(k, v)| JsonField { key: k.clone(), value: v.clone() }).collect())
}

fn field(key: &str, value: Json) -> JsonField {
    JsonField { key: key.into(), value }
}

// ---- Layout ----

fn rows_vec(l: &Layout) -> Vec<Vec<i64>> {
    l.rows.iter().map(|r| r.to_vec()).collect()
}

pub fn place(mut l: Layout, n: i64, r: i64) -> Result<Layout, String> {
    let mut rows = rows_vec(&l);
    let c = rows[r as usize].len() as i64;
    rows[r as usize].push(n);
    l.rows = rows.into_iter().map(List::from).collect();
    l.pos = l.pos.merge(&Map::from_iter([(n, Pos { r, c })]));
    Ok(l)
}

pub fn push_down(mut l: Layout, set: Map<i64, bool>) -> Result<Layout, String> {
    let rows: Vec<Vec<i64>> = rows_vec(&l).into_iter().map(|r| r.into_iter().filter(|n| !set.contains_key(n)).collect()).collect();
    let mut pos = std::collections::BTreeMap::new();
    for (ri, r) in rows.iter().enumerate() {
        for (ci, &n) in r.iter().enumerate() {
            pos.insert(n, Pos { r: ri as i64, c: ci as i64 });
        }
    }
    l.rows = rows.into_iter().map(List::from).collect();
    l.pos = pos.into();
    let keys: Vec<i64> = set.keys().copied().collect();
    l.out = l.out.without(&keys);
    Ok(l)
}



pub fn trim_rows(mut l: Layout) -> Result<Layout, String> {
    let mut rows = rows_vec(&l);
    while rows.last().is_some_and(|r| r.is_empty()) {
        rows.pop();
    }
    l.rows = rows.into_iter().map(List::from).collect();
    Ok(l)
}

/// Nodes in the last row with an edge out, in a region.
pub fn frontier(l: Layout, g: Graph) -> Result<List<i64>, String> {
    let last = l.rows.last().cloned().unwrap_or_default();
    Ok(last.iter().copied().filter(|n| g.edges.contains_key(n) && g.region_of.contains_key(n)).collect())
}

/// Targets of n's recorded forward edges.
pub fn forward(l: Layout, n: i64) -> Result<List<i64>, String> {
    Ok(l.out.get(&n).map(|es| es.iter().filter(|e| !e.back).map(|e| e.target).collect()).unwrap_or_default())
}

fn leaves(a: &SArm, path: &mut Vec<Str>, out: &mut Vec<(Vec<Str>, Call)>) {
    path.push(a.label.clone());
    match &a.body {
        SBody::Call(c) => out.push((path.clone(), c.clone())),
        SBody::Switch(arms) => arms.iter().for_each(|b| leaves(b, path, out)),
    }
    path.pop();
}

pub fn arm_paths(arms: List<SArm>) -> Result<List<ArmPathsOutItem>, String> {
    let mut out = vec![];
    for a in arms.iter() {
        leaves(a, &mut vec![], &mut out);
    }
    Ok(out.into_iter().map(|(labels, call)| ArmPathsOutItem { labels: labels.into(), call }).collect())
}

pub fn arm_label(s: Str) -> Result<Str, String> {
    let Some(inner) = s.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else { return Ok(s) };
    // Only if those two parentheses match each other: not `(a) || (b)`.
    let mut depth = 0i32;
    for c in inner.chars() {
        depth += match c {
            '(' => 1,
            ')' => -1,
            _ => 0,
        };
        if depth < 0 {
            return Ok(s);
        }
    }
    Ok(inner.trim().into())
}

pub fn join_labels(ls: List<Str>) -> Result<Str, String> {
    let ls: Vec<Str> = ls.iter().map(|l| arm_label(l.clone())).collect::<Result<_, _>>()?;
    Ok(ls.iter().map(|l| &**l).collect::<Vec<_>>().join(" → ").into())
}

/// All procs, nested ones included.
pub fn all_procs(nodes: List<NodeDecl>) -> Result<List<i64>, String> {
    Ok((0..nodes.len() as i64).filter(|&i| matches!(nodes[i as usize].kind, NodeKind::Proc(_))).collect())
}

pub fn graph_of(n: NodeDecl) -> Result<Graph, String> {
    match n.kind {
        NodeKind::Proc(p) => Ok(p.graph),
        _ => err("graph_of"),
    }
}


fn qual(n: &NodeDecl) -> String {
    n.qual.iter().map(|s| &**s).collect::<Vec<_>>().join(".")
}

fn display(prog: &Program, pid: i64, n: i64) -> String {
    let NodeKind::Proc(p) = &prog.nodes[pid as usize].kind else { return String::new() };
    if p.locals.contains(&n) { prog.nodes[n as usize].qual.last().map(|s| s.to_string()).unwrap_or_default() } else { qual(&prog.nodes[n as usize]) }
}

pub fn display_label(prog: Program, pid: i64, n: i64) -> Result<Str, String> {
    Ok(display(&prog, pid, n).into())
}

fn node_json(prog: &Program, pid: i64, n: i64) -> Json {
    let decl = &prog.nodes[n as usize];
    let (ty, label, sub) = match &decl.kind {
        NodeKind::Func(f) => {
            let imps: Vec<String> = f.imps.iter().map(|i| format!("{}:{}", i.file, i.function)).collect();
            (0, display(prog, pid, n), (!imps.is_empty()).then(|| imps.join(" · ")))
        }
        NodeKind::Proc(_) => (1, qual(decl), None),
        NodeKind::Id(_) => (0, "id".to_string(), None),
        NodeKind::Alias(b) => {
            let ty = if matches!(prog.nodes[*b as usize].kind, NodeKind::Proc(_)) { 1 } else { 0 };
            (ty, display(prog, pid, n), Some(format!("instance of {}", display(prog, pid, *b))))
        }
    };
    let mut fs = vec![field("type", Json::Num(ty)), field("label", Json::Str(label.into()))];
    if let Some(s) = sub {
        fs.push(field("sublabel", Json::Str(s.into())));
    }
    Json::Obj(fs.into())
}

pub fn rows_json(prog: Program, pid: i64, l: Layout) -> Result<Json, String> {
    Ok(Json::Arr(
        l.rows
            .iter()
            .map(|r| Json::Obj(vec![field("nodes", Json::Arr(r.iter().map(|&n| node_json(&prog, pid, n)).collect()))].into()))
            .collect(),
    ))
}

pub fn edges_json(l: Layout) -> Result<Json, String> {
    let mut out = vec![];
    for (ri, r) in l.rows.iter().enumerate() {
        for (ci, n) in r.iter().enumerate() {
            for e in l.out.get(n).map(|es| es.to_vec()).unwrap_or_default() {
                let Some(t) = l.pos.get(&e.target) else { continue };
                let mut fs = vec![
                    field("type", Json::Num(e.ty)),
                    field("source", Json::Arr(vec![Json::Num(ri as i64), Json::Num(ci as i64)].into())),
                    field("target", Json::Arr(vec![Json::Num(t.r), Json::Num(t.c)].into())),
                ];
                if let MaybeStr::Some(label) = &e.label {
                    fs.push(field("label", Json::Str(label.clone())));
                }
                fs.push(field("valueType", Json::Str(e.value.clone())));
                fs.push(field("call", Json::Str(e.call.clone())));
                out.push(Json::Obj(fs.into()));
            }
        }
    }
    Ok(Json::Arr(out.into()))
}
