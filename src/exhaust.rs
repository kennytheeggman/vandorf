//! Exhaustiveness and reachability of pattern switches.
//!
//! The usual pattern-matrix algorithm (Maranget, "Warnings for pattern
//! matching"): a pattern is *useful* after some rows if some value matches
//! it but none of the rows, and a switch is exhaustive if a wildcard isn't
//! useful after all its arms. Enums have one constructor per variant,
//! structs a single constructor, and other types none that patterns can
//! name, so only wildcards cover them.

use crate::sema::{Pat, PatKind};
use crate::types::{Type, TypeDef};

/// A pattern with bindings erased.
#[derive(Debug, Clone)]
enum P {
    Wild,
    Ctor(String, Vec<P>),
    Or(Vec<P>),
}

/// A value no row matches, for error messages.
#[derive(Debug, Clone)]
enum W {
    Wild,
    Ctor(String, Vec<W>),
}

const STRUCT: &str = "{}";

fn lower(p: &Pat) -> P {
    match &p.kind {
        PatKind::Wild | PatKind::Bind(_) => P::Wild,
        PatKind::Variant { tag, payload } => P::Ctor(tag.clone(), payload.iter().map(|q| lower(q)).collect()),
        PatKind::Struct(fs) => P::Ctor(STRUCT.into(), fs.iter().map(|(_, q)| lower(q)).collect()),
        PatKind::Or(alts) => P::Or(alts.iter().map(lower).collect()),
    }
}

struct Cx<'a> {
    defs: &'a [TypeDef],
}

impl<'a> Cx<'a> {
    /// Constructors of a type and their argument types, if patterns can
    /// name them.
    fn ctors(&self, t: &'a Type) -> Option<Vec<(String, Vec<&'a Type>)>> {
        match t.unfold(self.defs) {
            Type::Enum(vs) => Some(vs.iter().map(|(n, t)| (n.clone(), if t.is_void() { vec![] } else { vec![t] })).collect()),
            Type::Struct(fs) => Some(vec![(STRUCT.into(), fs.iter().map(|(_, t)| t).collect())]),
            _ => None,
        }
    }

    /// Rows with or-patterns in the first column split into one row each.
    fn expand(rows: &[Vec<P>]) -> Vec<Vec<P>> {
        let mut out = vec![];
        for r in rows {
            match &r[0] {
                P::Or(alts) => {
                    let split: Vec<Vec<P>> = alts.iter().map(|a| [vec![a.clone()], r[1..].to_vec()].concat()).collect();
                    out.extend(Self::expand(&split));
                }
                _ => out.push(r.clone()),
            }
        }
        out
    }

    /// Rows that match constructor `c` (with `arity` arguments), with the
    /// first column replaced by its arguments.
    fn specialize(rows: &[Vec<P>], c: &str, arity: usize) -> Vec<Vec<P>> {
        Self::expand(rows)
            .into_iter()
            .filter_map(|r| match &r[0] {
                P::Wild => Some([vec![P::Wild; arity], r[1..].to_vec()].concat()),
                P::Ctor(d, args) if d == c => Some([args.clone(), r[1..].to_vec()].concat()),
                _ => None,
            })
            .collect()
    }

    /// Rows whose first column is a wildcard, without it.
    fn default(rows: &[Vec<P>]) -> Vec<Vec<P>> {
        Self::expand(rows).into_iter().filter(|r| matches!(r[0], P::Wild)).map(|r| r[1..].to_vec()).collect()
    }

    fn used(rows: &[Vec<P>]) -> Vec<String> {
        Self::expand(rows)
            .iter()
            .filter_map(|r| match &r[0] {
                P::Ctor(c, _) => Some(c.clone()),
                _ => None,
            })
            .collect()
    }

    /// Is there a value matching `q` but no row?
    fn useful(&self, rows: &[Vec<P>], q: &[P], tys: &[&'a Type]) -> bool {
        let Some(first) = q.first() else { return rows.is_empty() };
        match first {
            P::Or(alts) => alts.iter().any(|a| self.useful(rows, &[vec![a.clone()], q[1..].to_vec()].concat(), tys)),
            P::Ctor(c, args) => {
                let arg_tys = self.ctors(tys[0]).and_then(|cs| cs.into_iter().find(|(d, _)| d == c)).map(|(_, a)| a).unwrap();
                self.useful(&Self::specialize(rows, c, args.len()), &[args.clone(), q[1..].to_vec()].concat(), &[arg_tys, tys[1..].to_vec()].concat())
            }
            P::Wild => {
                let used = Self::used(rows);
                match self.ctors(tys[0]) {
                    Some(cs) if cs.iter().all(|(c, _)| used.contains(c)) => cs.into_iter().any(|(c, arg_tys)| {
                        let n = arg_tys.len();
                        self.useful(&Self::specialize(rows, &c, n), &[vec![P::Wild; n], q[1..].to_vec()].concat(), &[arg_tys, tys[1..].to_vec()].concat())
                    }),
                    _ => self.useful(&Self::default(rows), &q[1..], &tys[1..]),
                }
            }
        }
    }

    /// Values (one per column) that no row matches, if any.
    fn missing(&self, rows: &[Vec<P>], tys: &[&'a Type]) -> Option<Vec<W>> {
        if tys.is_empty() {
            return rows.is_empty().then(Vec::new);
        }
        let used = Self::used(rows);
        match self.ctors(tys[0]) {
            Some(cs) if cs.iter().all(|(c, _)| used.contains(c)) => cs.into_iter().find_map(|(c, arg_tys)| {
                let n = arg_tys.len();
                let ws = self.missing(&Self::specialize(rows, &c, n), &[arg_tys, tys[1..].to_vec()].concat())?;
                Some([vec![W::Ctor(c, ws[..n].to_vec())], ws[n..].to_vec()].concat())
            }),
            cs => {
                let rest = self.missing(&Self::default(rows), &tys[1..])?;
                // Some constructor no row names, or any value of a type
                // without constructors.
                let head = match cs.and_then(|cs| cs.into_iter().find(|(c, _)| !used.contains(c))) {
                    Some((c, args)) => W::Ctor(c, vec![W::Wild; args.len()]),
                    None => W::Wild,
                };
                Some([vec![head], rest].concat())
            }
        }
    }

    fn render(&self, w: &W, t: &Type) -> String {
        match (w, t.unfold(self.defs)) {
            (W::Wild, _) => "_".into(),
            (W::Ctor(_, args), Type::Struct(fs)) => {
                let parts: Vec<String> = fs.iter().zip(args).map(|((n, t), a)| format!("{n} = {}", self.render(a, t))).collect();
                format!("{{ {} }}", parts.join(", "))
            }
            (W::Ctor(c, args), _) => {
                let payload = t.variant(c, self.defs).unwrap();
                format!("{c}({})", args.first().map(|a| self.render(a, payload)).unwrap_or_default())
            }
        }
    }
}

/// Indices of arms that can never match because earlier arms cover them.
pub fn unreachable(pats: &[&Pat], ty: &Type, defs: &[TypeDef]) -> Option<usize> {
    let cx = Cx { defs };
    let mut rows: Vec<Vec<P>> = vec![];
    for (i, p) in pats.iter().enumerate() {
        let row = vec![lower(p)];
        if !cx.useful(&rows, &row, &[ty]) {
            return Some(i);
        }
        rows.push(row);
    }
    None
}

/// A value of `ty` that none of `pats` matches, written as a pattern.
pub fn missing(pats: &[&Pat], ty: &Type, defs: &[TypeDef]) -> Option<String> {
    let cx = Cx { defs };
    let rows: Vec<Vec<P>> = pats.iter().map(|p| vec![lower(p)]).collect();
    cx.missing(&rows, &[ty]).map(|ws| cx.render(&ws[0], ty))
}
