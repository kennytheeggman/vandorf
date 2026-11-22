//! Document generation: lays each proc out as a grid of rows.
//!
//! The JSON shape matches the Go `ui/document` package, so the existing
//! visualizer can render it.
//!
//! Layout is a breadth-first sweep, one row per step: every node in the last
//! row contributes its edge targets to the next row. When an edge reaches a
//! node that is already placed (and is not a back edge), that node is pushed
//! down: it and everything placed downstream of it are removed from the grid,
//! the node is re-placed in the row being built, and its descendants are
//! re-discovered from there on later sweeps. So each node ends up below all of
//! its forward predecessors. An edge is a back edge, drawn upward without
//! moving anything, when its target already reaches its source through the
//! recorded forward edges (loops like `Loop -> Loop` or `End -> Start`).

use crate::sema::{Edge as SEdge, Global, Graph, NodeId, NodeKind, Program};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Serialize)]
pub struct Document {
    pub grids: BTreeMap<String, Grid>,
}

#[derive(Debug, Serialize, Default)]
pub struct Grid {
    pub rows: Vec<Row>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Serialize)]
pub struct Row {
    pub nodes: Vec<Node>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(into = "u8")]
pub enum NodeType {
    Func = 0,
    Proc = 1,
    Ellipsis = 2,
}

#[derive(Debug, Serialize)]
pub struct Node {
    #[serde(rename = "type")]
    pub ty: NodeType,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sublabel: Option<String>,
    /// The interface a func, or the func an instance is of, belongs to. Not
    /// in the Go document format.
    #[serde(rename = "interface", skip_serializing_if = "Option::is_none")]
    pub iface: Option<String>,
    /// A spawn branch: "joined" or "detached". Not in the Go document
    /// format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(into = "u8")]
pub enum EdgeType {
    Call = 0,
    Match = 1,
    ForEachExpand = 2,
    ForEachContinue = 3,
    ForEachCollect = 4,
    /// A spawner to one of its branches.
    Spawn = 5,
    /// A joined branch to the join.
    Join = 6,
    /// A spawn whose branches are all detached, on to the join: drawn from
    /// the strip, so it leaves from the first branch.
    Pass = 7,
}

#[derive(Debug, Serialize)]
pub struct Edge {
    #[serde(rename = "type")]
    pub ty: EdgeType,
    pub source: [usize; 2],
    pub target: [usize; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Type of `this` along the edge: what the target's arguments are
    /// computed from. Not in the Go document format.
    #[serde(rename = "valueType", skip_serializing_if = "Option::is_none")]
    pub value_type: Option<String>,
    /// What the target receives: its name and parameters, `B(i: int)`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
    /// The arguments as written, `a = this.a, b = 1`, or id's value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
}

impl From<NodeType> for u8 {
    fn from(t: NodeType) -> u8 {
        t as u8
    }
}
impl From<EdgeType> for u8 {
    fn from(t: EdgeType) -> u8 {
        t as u8
    }
}

/// One grid per proc, nested procs included, keyed by qualified name. A proc
/// node's label is its grid key, so the visualizer can expand it inline.
pub fn generate(prog: &Program) -> Document {
    let grids = prog
        .all_procs()
        .into_iter()
        .map(|p| (prog.node(p).qual_name(), layout(prog, p)))
        .collect();
    Document { grids }
}

struct Pending {
    ty: EdgeType,
    target: NodeId,
    label: Option<String>,
    value: String,
    call: String,
    args: String,
    back: bool,
}

/// An edge out of a node, before layout.
struct Out {
    ty: EdgeType,
    target: NodeId,
    label: Option<String>,
    /// Type of `this` along the edge.
    value: String,
    args: String,
}

#[derive(Default)]
struct Layout {
    rows: Vec<Vec<NodeId>>,
    pos: HashMap<NodeId, (usize, usize)>,
    /// Edges recorded from each placed node.
    out: HashMap<NodeId, Vec<Pending>>,
}

impl Layout {
    fn place(&mut self, n: NodeId, row: usize) {
        self.pos.insert(n, (row, self.rows[row].len()));
        self.rows[row].push(n);
    }

    fn remove(&mut self, n: NodeId) {
        let Some((r, c)) = self.pos.remove(&n) else { return };
        self.rows[r].remove(c);
        for &m in &self.rows[r][c..] {
            self.pos.get_mut(&m).unwrap().1 -= 1;
        }
    }

    /// `from` and every placed node reachable from it along forward edges.
    fn downstream(&self, from: NodeId) -> HashSet<NodeId> {
        let mut seen = HashSet::new();
        let mut stack = vec![from];
        while let Some(n) = stack.pop() {
            if !self.pos.contains_key(&n) || !seen.insert(n) {
                continue;
            }
            for e in self.out.get(&n).into_iter().flatten().filter(|e| !e.back) {
                stack.push(e.target);
            }
        }
        seen
    }

    /// Removes `set` from the grid and forgets their outgoing edges, so they
    /// are re-discovered (and re-recorded) below their new parent.
    fn push_down(&mut self, set: &HashSet<NodeId>) {
        for &n in set {
            self.remove(n);
            self.out.remove(&n);
        }
    }
}

fn edge_targets(prog: &Program, g: &Graph, src: NodeId) -> Vec<Out> {
    let in_region = g.region_of.get(&src).is_some_and(|&r| r != 0);
    let ret = prog.ret(src).to_string();
    // A branch leads on to its spawn's join.
    if let Some(sp) = g.spawned.get(&src) {
        let Some(SEdge::Spawn { branches, join, joined }) = g.edges.get(sp) else { unreachable!() };
        let b = branches.iter().find(|b| b.call.target == src).unwrap();
        let first = branches[0].call.target == src;
        let ty = match (b.detach, branches.iter().all(|b| b.detach)) {
            (false, _) => EdgeType::Join,
            (true, true) if first => EdgeType::Pass,
            (true, _) => return vec![],
        };
        return vec![Out { ty, target: join.target, label: None, value: joined.to_string(), args: join.text.clone() }];
    }
    match &g.edges[&src] {
        SEdge::Call(c) => {
            let ty = if in_region { EdgeType::ForEachContinue } else { EdgeType::Call };
            vec![Out { ty, target: c.target, label: None, value: ret, args: c.text.clone() }]
        }
        SEdge::Match(arms) => arms
            .iter()
            .flat_map(|a| a.leaves())
            .map(|(path, c)| {
                // A nested switch's arms are flattened into one edge per call.
                let label: Vec<String> = path.iter().map(|l| arm_label(l)).collect();
                Out { ty: EdgeType::Match, target: c.target, label: Some(label.join(" → ")), value: ret.clone(), args: c.text.clone() }
            })
            .collect(),
        SEdge::ForEach { call, elem, .. } => {
            vec![Out { ty: EdgeType::ForEachExpand, target: call.target, label: None, value: elem.to_string(), args: call.text.clone() }]
        }
        SEdge::Collect(c) => vec![Out { ty: EdgeType::ForEachCollect, target: c.target, label: None, value: format!("[]{ret}"), args: c.text.clone() }],
        SEdge::Spawn { branches, .. } => branches
            .iter()
            .map(|b| Out { ty: EdgeType::Spawn, target: b.call.target, label: None, value: ret.clone(), args: b.call.text.clone() })
            .collect(),
    }
}

/// `B(i: int, l: []int)`: what a call to `n` receives.
fn call_sig(prog: &Program, pid: NodeId, n: NodeId) -> String {
    let params: Vec<String> = prog.params(n).iter().map(|(p, t)| format!("{p}: {t}")).collect();
    format!("{}({})", display_label(prog, pid, n), params.join(", "))
}

/// An arm's head as written, without the parentheses around a condition.
fn arm_label(s: &str) -> String {
    let Some(inner) = s.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else { return s.to_string() };
    // Only if those two parentheses match each other: not `(a) || (b)`.
    let mut depth = 0i32;
    for c in inner.chars() {
        depth += match c {
            '(' => 1,
            ')' => -1,
            _ => 0,
        };
        if depth < 0 {
            return s.to_string();
        }
    }
    inner.trim().to_string()
}

fn layout(prog: &Program, pid: NodeId) -> Grid {
    let proc = prog.proc(pid);
    let g = &proc.graph;
    let has_edge = |n: &NodeId| (g.edges.contains_key(n) && g.region_of.contains_key(n)) || g.spawned.contains_key(n);

    let mut l = Layout { rows: vec![vec![]], ..Default::default() };
    l.place(proc.entry, 0);
    // Every pushdown moves a node strictly lower and forward edges form a
    // DAG, so the grid can't grow past one row per node. Guard regardless.
    let max_rows = g.region_of.len() + g.spawned.len() + 1;

    while l.rows.len() <= max_rows {
        let last = l.rows.len() - 1;
        let frontier: Vec<NodeId> = l.rows[last].iter().copied().filter(|n| has_edge(n)).collect();
        if frontier.is_empty() {
            break;
        }
        let row = l.rows.len();
        l.rows.push(vec![]);
        for src in frontier {
            // A pushdown earlier in this sweep may have moved src down a row
            // (it will be swept from there) or already recorded its edges.
            if l.pos.get(&src).map(|p| p.0) != Some(last) || l.out.contains_key(&src) {
                continue;
            }
            let mut recorded = vec![];
            for Out { ty, target, label, value, args } in edge_targets(prog, g, src) {
                let back = match l.pos.get(&target) {
                    None => {
                        l.place(target, row);
                        false
                    }
                    Some(&(r, _)) if r == row => false,
                    Some(_) => {
                        let down = l.downstream(target);
                        if target == src || down.contains(&src) {
                            true
                        } else {
                            l.push_down(&down);
                            l.place(target, row);
                            false
                        }
                    }
                };
                let call = call_sig(prog, pid, target);
                recorded.push(Pending { ty, target, label, value, call, args, back });
            }
            l.out.insert(src, recorded);
        }
    }
    while l.rows.last().is_some_and(|r| r.is_empty()) {
        l.rows.pop();
    }

    let node = |n: NodeId| {
        let decl = prog.node(n);
        match &decl.kind {
            NodeKind::Func(f) => Node {
                ty: NodeType::Func,
                label: display_label(prog, pid, n),
                sublabel: (!f.imps.is_empty()).then(|| {
                    f.imps.iter().map(|i| format!("{}:{}", i.file, i.function)).collect::<Vec<_>>().join(" · ")
                }),
                iface: interface_of(prog, n),
                branch: None,
            },
            NodeKind::Proc(_) => Node { ty: NodeType::Proc, label: decl.qual_name(), sublabel: None, iface: None, branch: None },
            NodeKind::Id(_) => Node { ty: NodeType::Func, label: "id".into(), sublabel: None, iface: None, branch: None },
            // An instance shows what it instances; its kind is that node's.
            NodeKind::Alias(b) => {
                let ty = if matches!(prog.node(*b).kind, NodeKind::Proc(_)) { NodeType::Proc } else { NodeType::Func };
                Node {
                    ty,
                    label: display_label(prog, pid, n),
                    sublabel: Some(format!("instance of {}", display_label(prog, pid, *b))),
                    iface: interface_of(prog, *b),
                    branch: None,
                }
            }
        }
    };
    // Spawn branches are marked joined or detached.
    let branch = |n: NodeId, mut node: Node| {
        if let Some(Some(SEdge::Spawn { branches, .. })) = g.spawned.get(&n).map(|s| g.edges.get(s)) {
            let detach = branches.iter().any(|b| b.call.target == n && b.detach);
            node.branch = Some(if detach { "detached" } else { "joined" });
        }
        node
    };
    let mut grid = Grid {
        rows: l.rows.iter().map(|r| Row { nodes: r.iter().map(|&n| branch(n, node(n))).collect() }).collect(),
        edges: vec![],
    };
    for (ri, r) in l.rows.iter().enumerate() {
        for (ci, n) in r.iter().enumerate() {
            for e in l.out.get(n).into_iter().flatten() {
                let Some(&(tr, tc)) = l.pos.get(&e.target) else { continue };
                grid.edges.push(Edge {
                    ty: e.ty,
                    source: [ri, ci],
                    target: [tr, tc],
                    label: e.label.clone(),
                    value_type: Some(e.value.clone()),
                    call: Some(e.call.clone()),
                    args: (!e.args.is_empty()).then(|| e.args.clone()),
                });
            }
        }
    }
    grid
}

/// Funcs local to the proc are shown by bare name; anything else (interface
/// funcs, top-level funcs) by qualified name.
fn display_label(prog: &Program, pid: NodeId, n: NodeId) -> String {
    if prog.proc(pid).locals.contains(&n) {
        prog.node(n).name().to_string()
    } else {
        prog.node(n).qual_name()
    }
}

/// The interface `n` is a func of, if any.
fn interface_of(prog: &Program, n: NodeId) -> Option<String> {
    match prog.node(n).qual.as_slice() {
        [i, _] if matches!(prog.globals.get(i), Some(Global::Interface(_))) => Some(i.clone()),
        _ => None,
    }
}
