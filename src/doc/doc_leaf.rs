//! Leaves for vcc's document layout (document.sys): grid updates and JSON.

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
    Ok(m.iter()
        .map(|(k, v)| JsonField {
            key: k.clone(),
            value: v.clone(),
        })
        .collect())
}

fn field(key: &str, value: Json) -> JsonField {
    JsonField {
        key: key.into(),
        value,
    }
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
    let rows: Vec<Vec<i64>> = rows_vec(&l)
        .into_iter()
        .map(|r| r.into_iter().filter(|n| !set.contains_key(n)).collect())
        .collect();
    let mut pos = std::collections::BTreeMap::new();
    for (ri, r) in rows.iter().enumerate() {
        for (ci, &n) in r.iter().enumerate() {
            pos.insert(
                n,
                Pos {
                    r: ri as i64,
                    c: ci as i64,
                },
            );
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
    Ok(last
        .iter()
        .copied()
        .filter(|n| {
            (g.edges.contains_key(n) && g.region_of.contains_key(n)) || g.spawned.contains_key(n)
        })
        .collect())
}

fn spawn_of(g: &Graph, sp: i64) -> &SEdgeSpawn {
    match g.edges.get(&sp) {
        Some(SEdge::Spawn(s)) => s,
        _ => unreachable!(),
    }
}

/// A spawner's edges, one to each branch.
pub fn spawn_outs(branches: List<SBranch>, value: Str) -> Result<List<Out>, String> {
    Ok(branches
        .iter()
        .map(|b| Out {
            ty: 5,
            target: b.call.target,
            label: MaybeStr::None,
            value: value.clone(),
            args: b.call.text.clone(),
        })
        .collect())
}

/// A branch's edge on to its spawn's join: a joined one's, or, when every
/// branch is detached, the first one's (drawn from the strip); none for
/// other detached ones.
pub fn branch_outs(g: Graph, src: i64, value: Str) -> Result<List<Out>, String> {
    let s = spawn_of(&g, g.spawned[&src]);
    let b = s.branches.iter().find(|b| b.call.target == src).unwrap();
    let first = s.branches[0].call.target == src;
    let ty = match (b.detach, s.branches.iter().all(|b| b.detach)) {
        (false, _) => 6,
        (true, true) if first => 7,
        (true, _) => return Ok(List::default()),
    };
    Ok(vec![Out {
        ty,
        target: s.join.target,
        label: MaybeStr::None,
        value,
        args: s.join.text.clone(),
    }]
    .into())
}

pub fn joined_type(g: Graph, src: i64) -> Result<Type, String> {
    Ok(spawn_of(&g, g.spawned[&src]).joined.clone())
}

/// Targets of n's recorded forward edges.
pub fn forward(l: Layout, n: i64) -> Result<List<i64>, String> {
    Ok(l.out
        .get(&n)
        .map(|es| es.iter().filter(|e| !e.back).map(|e| e.target).collect())
        .unwrap_or_default())
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
    Ok(out
        .into_iter()
        .map(|(labels, call)| ArmPathsOutItem {
            labels: labels.into(),
            call,
        })
        .collect())
}

pub fn arm_label(s: Str) -> Result<Str, String> {
    let Some(inner) = s.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
        return Ok(s);
    };
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
    let ls: Vec<Str> = ls
        .iter()
        .map(|l| arm_label(l.clone()))
        .collect::<Result<_, _>>()?;
    Ok(ls
        .iter()
        .map(|l| &**l)
        .collect::<Vec<_>>()
        .join(" → ")
        .into())
}

/// All procs, nested ones included.
pub fn all_procs(nodes: List<NodeDecl>) -> Result<List<i64>, String> {
    Ok((0..nodes.len() as i64)
        .filter(|&i| matches!(nodes[i as usize].kind, NodeKind::Proc(_)))
        .collect())
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
    let NodeKind::Proc(p) = &prog.nodes[pid as usize].kind else {
        return String::new();
    };
    if p.locals.contains(&n) {
        prog.nodes[n as usize]
            .qual
            .last()
            .map(|s| s.to_string())
            .unwrap_or_default()
    } else {
        qual(&prog.nodes[n as usize])
    }
}

pub fn display_label(prog: Program, pid: i64, n: i64) -> Result<Str, String> {
    Ok(display(&prog, pid, n).into())
}

/// The interface n is a func of, if any.
fn interface_of(prog: &Program, n: i64) -> Option<String> {
    match &prog.nodes[n as usize].qual[..] {
        [i, _] if matches!(prog.globals.get(i), Some(Global::Interface(_))) => Some(i.to_string()),
        _ => None,
    }
}

/// `id1`, `id2`, ...: an id(..) without `as`.
fn is_unnamed_id(name: &str) -> bool {
    name.strip_prefix("id")
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// A type as written.
fn show_ty(t: &Type) -> String {
    let members = |ms: &List<Member>, variants: bool| {
        ms.iter()
            .map(|m| match (variants, &m.ty) {
                (true, Type::Prim(p)) if &**p == "void" => m.name.to_string(),
                (true, ty) => format!("{}({})", m.name, show_ty(ty)),
                (false, ty) => format!("{}: {}", m.name, show_ty(ty)),
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    match t {
        Type::Prim(p) => p.to_string(),
        Type::Array(e) => format!("[]{}", show_ty(e)),
        Type::Map(m) => format!("map[{}]{}", show_ty(&m.key), show_ty(&m.value)),
        Type::Struct(ms) => format!("{{ {} }}", members(ms, false)),
        Type::Enum(ms) => format!("enum {{ {} }}", members(ms, true)),
        Type::Named(n) => n.name.to_string(),
    }
}

/// A signature's parameters as written: `(a: int, b: []string)`.
fn show_params(ps: &List<Member>) -> String {
    let shown: Vec<String> = ps
        .iter()
        .map(|m| format!("{}: {}", m.name, show_ty(&m.ty)))
        .collect();
    format!("({})", shown.join(", "))
}

fn node_json(prog: &Program, pid: i64, n: i64) -> Json {
    let decl = &prog.nodes[n as usize];
    let mut iface = None;
    let (ty, label, sub) = match &decl.kind {
        NodeKind::Func(f) => {
            let imps: Vec<String> = f
                .imps
                .iter()
                .map(|i| format!("{}:{}", i.file, i.function))
                .collect();
            iface = interface_of(prog, n);
            (
                0,
                display(prog, pid, n),
                (!imps.is_empty()).then(|| imps.join(" · ")),
            )
        }
        NodeKind::Proc(_) => (1, qual(decl), None),
        NodeKind::Id(ps) if decl.qual.last().is_some_and(|l| is_unnamed_id(l)) || ps.is_empty() => {
            (0, "id".to_string(), None)
        }
        // `id(..) as N`: a node named N.
        NodeKind::Id(_) => (0, display(prog, pid, n), Some("id".to_string())),
        // The proc's signature node (type 3): drawn as its header, with its
        // params.
        NodeKind::Sig(s) => (
            3,
            decl.qual.last().map_or(String::new(), |l| l.to_string()),
            Some(show_params(&s.params)),
        ),
        NodeKind::Alias(b) => {
            let ty = if matches!(prog.nodes[*b as usize].kind, NodeKind::Proc(_)) {
                1
            } else {
                0
            };
            iface = interface_of(prog, *b);
            (
                ty,
                display(prog, pid, n),
                Some(format!("instance of {}", display(prog, pid, *b))),
            )
        }
    };
    let mut fs = vec![
        field("type", Json::Num(ty)),
        field("label", Json::Str(label.into())),
    ];
    if let Some(s) = sub {
        fs.push(field("sublabel", Json::Str(s.into())));
    }
    if let Some(i) = iface {
        fs.push(field("interface", Json::Str(i.into())));
    }
    // Its declaration's doc comment: an instance's is what it instances'.
    let doc = match &decl.kind {
        NodeKind::Alias(b) => prog.nodes[*b as usize].doc.clone(),
        _ => decl.doc.clone(),
    };
    if !doc.is_empty() {
        fs.push(field("doc", Json::Str(doc)));
    }
    // Spawn branches are marked joined or detached.
    if let NodeKind::Proc(p) = &prog.nodes[pid as usize].kind
        && let Some(&sp) = p.graph.spawned.get(&n)
    {
        let detach = spawn_of(&p.graph, sp)
            .branches
            .iter()
            .any(|b| b.call.target == n && b.detach);
        fs.push(field(
            "branch",
            Json::Str(if detach { "detached" } else { "joined" }.into()),
        ));
    }
    Json::Obj(fs.into())
}

pub fn rows_json(prog: Program, pid: i64, l: Layout) -> Result<Json, String> {
    Ok(Json::Arr(
        l.rows
            .iter()
            .map(|r| {
                Json::Obj(
                    vec![field(
                        "nodes",
                        Json::Arr(r.iter().map(|&n| node_json(&prog, pid, n)).collect()),
                    )]
                    .into(),
                )
            })
            .collect(),
    ))
}

pub fn edges_json(l: Layout) -> Result<Json, String> {
    let mut out = vec![];
    for (ri, r) in l.rows.iter().enumerate() {
        for (ci, n) in r.iter().enumerate() {
            for e in l.out.get(n).map(|es| es.to_vec()).unwrap_or_default() {
                let Some(t) = l.pos.get(&e.target) else {
                    continue;
                };
                let mut fs = vec![
                    field("type", Json::Num(e.ty)),
                    field(
                        "source",
                        Json::Arr(vec![Json::Num(ri as i64), Json::Num(ci as i64)].into()),
                    ),
                    field(
                        "target",
                        Json::Arr(vec![Json::Num(t.r), Json::Num(t.c)].into()),
                    ),
                ];
                if let MaybeStr::Some(label) = &e.label {
                    fs.push(field("label", Json::Str(label.clone())));
                }
                fs.push(field("valueType", Json::Str(e.value.clone())));
                fs.push(field("call", Json::Str(e.call.clone())));
                if !e.args.is_empty() {
                    fs.push(field("args", Json::Str(e.args.clone())));
                }
                out.push(Json::Obj(fs.into()));
            }
        }
    }
    Ok(Json::Arr(out.into()))
}

// ---- JSON for the IDE: compact, keys sorted (as serde_json's Value) ----

fn write_compact(j: &Json, out: &mut String) {
    match j {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Num(i) => out.push_str(&i.to_string()),
        Json::Str(s) => escape(s, out),
        Json::Arr(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_compact(x, out);
            }
            out.push(']');
        }
        Json::Obj(fs) => {
            let mut fs: Vec<&JsonField> = fs.iter().collect();
            fs.sort_by(|a, b| a.key.cmp(&b.key));
            out.push('{');
            for (i, f) in fs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                escape(&f.key, out);
                out.push(':');
                write_compact(&f.value, out);
            }
            out.push('}');
        }
    }
}

pub fn compact(j: Json) -> Result<Str, String> {
    let mut out = String::new();
    write_compact(&j, &mut out);
    Ok(out.into())
}

/// A JSON text as a Json; numbers that aren't ints are an error.
pub fn parse_json(text: Str) -> Result<ResJson, String> {
    struct P<'a> {
        b: &'a [u8],
        i: usize,
    }
    impl P<'_> {
        fn ws(&mut self) {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }
        fn value(&mut self) -> Result<Json, String> {
            self.ws();
            match self.b.get(self.i) {
                None => Err("EOF while parsing a value".into()),
                Some(b'n') if self.b[self.i..].starts_with(b"null") => {
                    self.i += 4;
                    Ok(Json::Null)
                }
                Some(b't') if self.b[self.i..].starts_with(b"true") => {
                    self.i += 4;
                    Ok(Json::Bool(true))
                }
                Some(b'f') if self.b[self.i..].starts_with(b"false") => {
                    self.i += 5;
                    Ok(Json::Bool(false))
                }
                Some(b'"') => Ok(Json::Str(self.string()?.into())),
                Some(b'[') => {
                    self.i += 1;
                    let mut xs = vec![];
                    self.ws();
                    if self.b.get(self.i) == Some(&b']') {
                        self.i += 1;
                        return Ok(Json::Arr(xs.into()));
                    }
                    loop {
                        xs.push(self.value()?);
                        self.ws();
                        match self.b.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b']') => {
                                self.i += 1;
                                return Ok(Json::Arr(xs.into()));
                            }
                            _ => return Err(format!("expected `,` or `]` at {}", self.i)),
                        }
                    }
                }
                Some(b'{') => {
                    self.i += 1;
                    let mut fs = vec![];
                    self.ws();
                    if self.b.get(self.i) == Some(&b'}') {
                        self.i += 1;
                        return Ok(Json::Obj(fs.into()));
                    }
                    loop {
                        self.ws();
                        let k = self.string()?;
                        self.ws();
                        if self.b.get(self.i) != Some(&b':') {
                            return Err(format!("expected `:` at {}", self.i));
                        }
                        self.i += 1;
                        fs.push(JsonField {
                            key: k.into(),
                            value: self.value()?,
                        });
                        self.ws();
                        match self.b.get(self.i) {
                            Some(b',') => self.i += 1,
                            Some(b'}') => {
                                self.i += 1;
                                return Ok(Json::Obj(fs.into()));
                            }
                            _ => return Err(format!("expected `,` or `}}` at {}", self.i)),
                        }
                    }
                }
                Some(c) if *c == b'-' || c.is_ascii_digit() => {
                    let start = self.i;
                    self.i += 1;
                    while self.i < self.b.len()
                        && matches!(
                            self.b[self.i],
                            b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'
                        )
                    {
                        self.i += 1;
                    }
                    let t = String::from_utf8_lossy(&self.b[start..self.i]).into_owned();
                    t.parse()
                        .map(Json::Num)
                        .map_err(|_| format!("number {t} isn't an int"))
                }
                Some(_) => Err(format!("expected value at {}", self.i)),
            }
        }
        fn string(&mut self) -> Result<String, String> {
            if self.b.get(self.i) != Some(&b'"') {
                return Err(format!("expected a string at {}", self.i));
            }
            self.i += 1;
            let mut out = Vec::new();
            loop {
                match self.b.get(self.i) {
                    None => return Err("EOF while parsing a string".into()),
                    Some(b'"') => {
                        self.i += 1;
                        return String::from_utf8(out).map_err(|e| e.to_string());
                    }
                    Some(b'\\') => {
                        let c = *self.b.get(self.i + 1).ok_or("EOF while parsing a string")?;
                        self.i += 2;
                        match c {
                            b'n' => out.push(b'\n'),
                            b't' => out.push(b'\t'),
                            b'r' => out.push(b'\r'),
                            b'b' => out.push(8),
                            b'f' => out.push(12),
                            b'u' => {
                                let hex = std::str::from_utf8(
                                    self.b.get(self.i..self.i + 4).ok_or("bad escape")?,
                                )
                                .map_err(|e| e.to_string())?;
                                let mut cp =
                                    u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                                self.i += 4;
                                // A surrogate pair.
                                if (0xd800..0xdc00).contains(&cp)
                                    && self.b[self.i..].starts_with(b"\\u")
                                {
                                    let lo = std::str::from_utf8(
                                        self.b.get(self.i + 2..self.i + 6).ok_or("bad escape")?,
                                    )
                                    .map_err(|e| e.to_string())?;
                                    let lo =
                                        u32::from_str_radix(lo, 16).map_err(|e| e.to_string())?;
                                    self.i += 6;
                                    cp = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                                }
                                out.extend_from_slice(
                                    char::from_u32(cp)
                                        .unwrap_or('\u{fffd}')
                                        .to_string()
                                        .as_bytes(),
                                );
                            }
                            c => out.push(c),
                        }
                    }
                    Some(&c) => {
                        out.push(c);
                        self.i += 1;
                    }
                }
            }
        }
    }
    let mut p = P {
        b: text.as_bytes(),
        i: 0,
    };
    let r = p.value().and_then(|v| {
        p.ws();
        if p.i == p.b.len() {
            Ok(v)
        } else {
            Err(format!("trailing characters at {}", p.i))
        }
    });
    Ok(match r {
        Ok(v) => ResJson::Ok(v),
        Err(e) => ResJson::Err(Diag {
            msg: e.into(),
            span: Span {
                at: 0,
                end: 0,
                file: 0,
            },
        }),
    })
}

/// A string field of a JSON object, if it is one.
pub fn json_str(j: Json, key: Str) -> Result<MaybeStr, String> {
    Ok(match j {
        Json::Obj(fs) => fs
            .iter()
            .find(|f| f.key == key)
            .and_then(|f| match &f.value {
                Json::Str(s) => Some(MaybeStr::Some(s.clone())),
                _ => None,
            })
            .unwrap_or(MaybeStr::None),
        _ => MaybeStr::None,
    })
}
