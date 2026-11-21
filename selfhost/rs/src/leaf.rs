//! vcc's leaf funcs. Each is pure: its result depends only on its
//! arguments, and it does no I/O. Errors are values too: a leaf fails with a
//! message, which fails the proc that called it.

use std::sync::Arc;

use crate::vcc::types::*;

const KEYWORDS: &[&str] = &[
    "interface", "func", "proc", "enum", "switch", "collect", "for", "each", "as", "type", "true", "false",
    "else", "map", "in", "id",
];

pub fn same(s: Str) -> Result<Str, String> {
    Ok(s)
}

// ---- Tokens ----

/// Multi-character symbols, longest first.
const SYMS: &[&str] = &["->", "==", "!=", "<=", ">=", "&&", "||"];

fn tok(kind: &str, text: &str, at: usize, end: usize) -> Tok {
    Tok { kind: kind.into(), text: text.into(), at: at as i64, end: end as i64 }
}

/// Splits `src` into tokens, ending with an "eof" token. `//` starts a
/// comment.
pub fn lex(src: Str) -> Result<List<Tok>, String> {
    let b = src.as_bytes();
    let mut toks = vec![];
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c.is_ascii_whitespace() {
            i += 1;
        } else if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            toks.push(tok("ident", &src[start..i], start, i));
        } else if c.is_ascii_digit() {
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let mut kind = "int";
            if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
                kind = "float";
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            toks.push(tok(kind, &src[start..i], start, i));
        } else if c == b'"' {
            let mut text = String::new();
            i += 1;
            loop {
                match src[i..].chars().next() {
                    None | Some('\n') => return Err(format!("{start}: unterminated string")),
                    Some('"') => {
                        i += 1;
                        break;
                    }
                    Some('\\') => {
                        text.push(match src[i + 1..].chars().next() {
                            Some('\\') => '\\',
                            Some('"') => '"',
                            Some('n') => '\n',
                            Some('t') => '\t',
                            _ => return Err(format!("{i}: unknown escape")),
                        });
                        i += 2;
                    }
                    Some(ch) => {
                        text.push(ch);
                        i += ch.len_utf8();
                    }
                }
            }
            toks.push(tok("str", &text, start, i));
        } else if let Some(s) = SYMS.iter().find(|s| src[i..].starts_with(**s)) {
            i += s.len();
            toks.push(tok("sym", s, start, i));
        } else if "(){}[]<>,;:.=+-*/%!|".contains(c as char) {
            i += 1;
            toks.push(tok("sym", &src[start..i], start, i));
        } else {
            let ch = src[i..].chars().next().unwrap();
            return Err(format!("{i}: unexpected character {ch:?}"));
        }
    }
    toks.push(tok("eof", "", b.len(), b.len()));
    Ok(toks.into())
}

// ---- Parser steps ----

fn get(toks: &[Tok], pos: i64) -> Tok {
    usize::try_from(pos).ok().and_then(|p| toks.get(p)).or(toks.last()).cloned().unwrap()
}

pub fn at(toks: List<Tok>, pos: i64) -> Result<Cur, String> {
    let (tok_, prev) = (get(&toks, pos), if pos > 0 { get(&toks, pos - 1) } else { tok("sof", "", 0, 0) });
    Ok(Cur { toks, pos, tok: tok_, prev })
}

pub fn at_level(toks: List<Tok>, pos: i64, level: i64) -> Result<AtLevel, String> {
    Ok(AtLevel { cur: at(toks, pos)?, level })
}

pub fn at_level_value(toks: List<Tok>, pos: i64, level: i64, left: Value) -> Result<AtLevelValue, String> {
    Ok(AtLevelValue { cur: at(toks, pos)?, level, left })
}

// Loop heads: the token at `pos`, and what the loop has gathered so far.

pub fn at_file(toks: List<Tok>, pos: i64, acc: File) -> Result<AtAccFile, String> {
    Ok(AtAccFile { cur: at(toks, pos)?, acc })
}

pub fn at_funcs(toks: List<Tok>, pos: i64, acc: List<Func>) -> Result<AtAccFuncList, String> {
    Ok(AtAccFuncList { cur: at(toks, pos)?, acc })
}

pub fn at_impls(toks: List<Tok>, pos: i64, acc: List<ImplRef>) -> Result<AtAccImplRefList, String> {
    Ok(AtAccImplRefList { cur: at(toks, pos)?, acc })
}

pub fn at_body(toks: List<Tok>, pos: i64, acc: BodyItems) -> Result<AtAccBodyItems, String> {
    Ok(AtAccBodyItems { cur: at(toks, pos)?, acc })
}

pub fn at_fields(toks: List<Tok>, pos: i64, acc: List<Field>) -> Result<AtAccFieldList, String> {
    Ok(AtAccFieldList { cur: at(toks, pos)?, acc })
}

pub fn at_variants(toks: List<Tok>, pos: i64, acc: List<Variant>) -> Result<AtAccVariantList, String> {
    Ok(AtAccVariantList { cur: at(toks, pos)?, acc })
}

pub fn at_types(toks: List<Tok>, pos: i64, acc: List<TypeExpr>) -> Result<AtAccTypeExprList, String> {
    Ok(AtAccTypeExprList { cur: at(toks, pos)?, acc })
}

pub fn at_segs(toks: List<Tok>, pos: i64, acc: List<Ident>) -> Result<AtAccIdentList, String> {
    Ok(AtAccIdentList { cur: at(toks, pos)?, acc })
}

pub fn at_args(toks: List<Tok>, pos: i64, acc: List<Arg>) -> Result<AtAccArgList, String> {
    Ok(AtAccArgList { cur: at(toks, pos)?, acc })
}

pub fn at_arms(toks: List<Tok>, pos: i64, acc: List<Arm>) -> Result<AtAccArmList, String> {
    Ok(AtAccArmList { cur: at(toks, pos)?, acc })
}

pub fn at_pats(toks: List<Tok>, pos: i64, acc: List<Pat>) -> Result<AtAccPatList, String> {
    Ok(AtAccPatList { cur: at(toks, pos)?, acc })
}

pub fn at_pat_fields(toks: List<Tok>, pos: i64, acc: List<PatField>) -> Result<AtAccPatFieldList, String> {
    Ok(AtAccPatFieldList { cur: at(toks, pos)?, acc })
}

pub fn at_values(toks: List<Tok>, pos: i64, acc: List<Value>) -> Result<AtAccValueList, String> {
    Ok(AtAccValueList { cur: at(toks, pos)?, acc })
}

pub fn at_entries(toks: List<Tok>, pos: i64, acc: List<Entry>) -> Result<AtAccEntryList, String> {
    Ok(AtAccEntryList { cur: at(toks, pos)?, acc })
}

pub fn at_apply_args(toks: List<Tok>, pos: i64, acc: List<ApplyArg>) -> Result<AtAccApplyArgList, String> {
    Ok(AtAccApplyArgList { cur: at(toks, pos)?, acc })
}

pub fn at_value(toks: List<Tok>, pos: i64, left: Value) -> Result<AtValue, String> {
    Ok(AtValue { cur: at(toks, pos)?, left })
}

fn shown(t: &Tok) -> String {
    match &*t.kind {
        "eof" => "end of input".into(),
        "str" => format!("{:?}", &*t.text),
        _ => format!("'{}'", t.text),
    }
}

/// Matches `toks[pos]` and moves past it; the error says what was wanted,
/// prefixed with the token's byte range ("at:end: ").
fn matched(toks: List<Tok>, pos: i64, ok: impl Fn(&Tok) -> bool, what: &str) -> Result<Cur, String> {
    let t = get(&toks, pos);
    if !ok(&t) {
        return Err(format!("{}:{}: expected {what}, found {}", t.at, t.end, shown(&t)));
    }
    Ok(Cur { toks, pos: pos + 1, tok: t.clone(), prev: t })
}

pub fn expect(toks: List<Tok>, pos: i64, text: Str, what: Str) -> Result<Cur, String> {
    matched(toks, pos, |t| (&*t.kind == "sym" || &*t.kind == "ident") && t.text == text, &what)
}

pub fn close_angle(toks: List<Tok>, pos: i64) -> Result<Cur, String> {
    let t = get(&toks, pos);
    if &*t.kind == "sym" && &*t.text == ">=" {
        return Err(format!("{}:{}: write `> =` here: `>=` is the comparison operator", t.at, t.end));
    }
    matched(toks, pos, |t| &*t.kind == "sym" && &*t.text == ">", "> to close the type arguments")
}

pub fn need_ident(toks: List<Tok>, pos: i64, what: Str) -> Result<Cur, String> {
    matched(toks, pos, |t| &*t.kind == "ident" && !KEYWORDS.contains(&&*t.text), &what)
}

pub fn need_name(toks: List<Tok>, pos: i64, what: Str) -> Result<Cur, String> {
    matched(toks, pos, |t| &*t.kind == "ident", &what)
}

pub fn need_str(toks: List<Tok>, pos: i64, what: Str) -> Result<Cur, String> {
    matched(toks, pos, |t| &*t.kind == "str", &what)
}

pub fn is_sym(t: Tok, s: Str) -> Result<bool, String> {
    Ok(&*t.kind == "sym" && t.text == s)
}

pub fn is_word(t: Tok, s: Str) -> Result<bool, String> {
    Ok(&*t.kind == "ident" && t.text == s)
}

pub fn is_close(t: Tok) -> Result<bool, String> {
    Ok(&*t.kind == "sym" && matches!(&*t.text, ")" | "}" | "]"))
}

// ---- Syntax ----

pub fn span_of(t: Tok) -> Result<Span, String> {
    Ok(Span { at: t.at, end: t.end })
}

pub fn mk_ident(t: Tok) -> Result<Ident, String> {
    Ok(Ident { name: t.text, span: Span { at: t.at, end: t.end } })
}

pub fn path_end(t: Tok, segs: List<Ident>) -> Result<i64, String> {
    Ok(segs.last().map_or(t.end, |s| s.span.end))
}

pub fn int_of(t: Tok) -> Result<i64, String> {
    t.text.parse().map_err(|_| format!("{}:{}: number {} is out of range", t.at, t.end, t.text))
}

pub fn float_of(t: Tok) -> Result<f64, String> {
    t.text.parse().map_err(|_| format!("{}:{}: number {} is out of range", t.at, t.end, t.text))
}

pub fn impl_of(t: Tok) -> Result<ImplRef, String> {
    let bad = || format!("{}:{}: expected \"file:function\", found {:?}", t.at, t.end, &*t.text);
    let (file, function) = t.text.rsplit_once(':').ok_or_else(bad)?;
    let is_ident = function.chars().enumerate().all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()));
    if file.is_empty() || function.is_empty() || !is_ident {
        return Err(bad());
    }
    Ok(ImplRef { file: file.into(), function: function.into(), span: Span { at: t.at, end: t.end } })
}

pub fn op_at(t: Tok, level: i64) -> Result<bool, String> {
    let ops: &[&str] = match level {
        0 => &["||"],
        1 => &["&&"],
        2 => &["==", "!=", "<", "<=", ">", ">="],
        3 => &["+", "-"],
        4 => &["*", "/", "%"],
        _ => &[],
    };
    Ok((&*t.kind == "sym" && ops.contains(&&*t.text)) || (level == 2 && &*t.kind == "ident" && &*t.text == "in"))
}

pub fn combine(op: Tok, l: Value, r: Value) -> Result<Value, String> {
    let span = Span { at: l.span.at, end: r.span.end };
    let kind = if &*op.text == "in" {
        ValueKind::In(Arc::new(ValueKindIn { key: Arc::new(l), map: Arc::new(r) }))
    } else {
        ValueKind::Binary(Arc::new(ValueKindBinary { op: op.text, l: Arc::new(l), r: Arc::new(r) }))
    };
    Ok(Value { kind: Arc::new(kind), span })
}

pub fn prefix(op: Tok, e: Value) -> Result<Value, String> {
    let span = Span { at: op.at, end: e.span.end };
    let kind = match (&*op.text, &*e.kind) {
        ("-", ValueKind::Int(i)) => ValueKind::Int(-i),
        ("-", ValueKind::Float(f)) => ValueKind::Float(-f),
        _ => ValueKind::Unary(Arc::new(ValueKindUnary { op: op.text.clone(), e: Arc::new(e) })),
    };
    Ok(Value { kind: Arc::new(kind), span })
}

pub fn or_of(pats: List<Pat>) -> Result<Pat, String> {
    if pats.len() == 1 {
        return Ok(pats[0].clone());
    }
    let span = Span { at: pats[0].span.at, end: pats.last().unwrap().span.end };
    Ok(Pat { kind: Arc::new(PatKind::Or(pats)), span })
}

pub fn summary(f: File) -> Result<Str, String> {
    let edges: usize = f.nodes.iter().map(count_edges).sum();
    Ok(format!("{} types, {} interfaces, {} nodes, {edges} edges", f.types.len(), f.interfaces.len(), f.nodes.len()).into())
}

fn count_edges(n: &Node) -> usize {
    match n {
        Node::Proc(p) => p.edges.len() + p.nodes.iter().map(count_edges).sum::<usize>(),
        Node::Func(_) => 0,
    }
}
