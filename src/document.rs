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

use crate::sema::{Edge as SEdge, Graph, NodeId, NodeKind, Program};
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
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(into = "u8")]
pub enum EdgeType {
    Call = 0,
    Match = 1,
    ForEachExpand = 2,
    ForEachContinue = 3,
    ForEachCollect = 4,
}

#[derive(Debug, Serialize)]
pub struct Edge {
    #[serde(rename = "type")]
    pub ty: EdgeType,
    pub source: [usize; 2],
    pub target: [usize; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
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
    back: bool,
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

fn edge_targets(g: &Graph, src: NodeId) -> Vec<(EdgeType, NodeId, Option<String>)> {
    let in_region = g.region_of.get(&src).is_some_and(|&r| r != 0);
    match &g.edges[&src] {
        SEdge::Call(c) => {
            let ty = if in_region { EdgeType::ForEachContinue } else { EdgeType::Call };
            vec![(ty, c.target, None)]
        }
        SEdge::Match(arms) => arms
            .iter()
            .map(|(cond, c)| (EdgeType::Match, c.target, Some(cond.clone())))
            .collect(),
        SEdge::ForEach { call, .. } => vec![(EdgeType::ForEachExpand, call.target, None)],
        SEdge::Collect(c) => vec![(EdgeType::ForEachCollect, c.target, None)],
    }
}

fn layout(prog: &Program, pid: NodeId) -> Grid {
    let proc = prog.proc(pid);
    let g = &proc.graph;
    let has_edge = |n: &NodeId| g.edges.contains_key(n) && g.region_of.contains_key(n);

    let mut l = Layout { rows: vec![vec![]], ..Default::default() };
    l.place(proc.entry, 0);
    // Every pushdown moves a node strictly lower and forward edges form a
    // DAG, so the grid can't grow past one row per node. Guard regardless.
    let max_rows = g.region_of.len() + 1;

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
            for (ty, target, label) in edge_targets(g, src) {
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
                recorded.push(Pending { ty, target, label, back });
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
            },
            NodeKind::Proc(_) => Node { ty: NodeType::Proc, label: decl.qual_name(), sublabel: None },
        }
    };
    let mut grid = Grid {
        rows: l.rows.iter().map(|r| Row { nodes: r.iter().map(|&n| node(n)).collect() }).collect(),
        edges: vec![],
    };
    for (ri, r) in l.rows.iter().enumerate() {
        for (ci, n) in r.iter().enumerate() {
            for e in l.out.get(n).into_iter().flatten() {
                let Some(&(tr, tc)) = l.pos.get(&e.target) else { continue };
                grid.edges.push(Edge { ty: e.ty, source: [ri, ci], target: [tr, tc], label: e.label.clone() });
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
