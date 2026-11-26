//! vcc's leaf funcs. Each is pure: its result depends only on its
//! arguments, and it does no I/O. Errors are values too: a leaf fails with a
//! message, which fails the proc that called it.

use std::sync::Arc;

use crate::vcc::types::*;

const KEYWORDS: &[&str] = &[
    "interface",
    "func",
    "proc",
    "enum",
    "switch",
    "collect",
    "for",
    "each",
    "as",
    "type",
    "true",
    "false",
    "else",
    "map",
    "in",
    "id",
];

pub fn fail(msg: Str) -> Result<Str, String> {
    Err(msg.to_string())
}

// ---- Tokens ----

/// Multi-character symbols, longest first.
const SYMS: &[&str] = &["->", "==", "!=", "<=", ">=", "&&", "||", ".."];

fn tok(kind: &str, text: &str, at: usize, end: usize, file: i64) -> Tok {
    Tok {
        kind: kind.into(),
        text: text.into(),
        at: at as i64,
        end: end as i64,
        file,
        doc: "".into(),
    }
}

/// Scans `src` into tokens: names, numbers, decoded strings and symbols (the
/// longest first), ending with an "eof" token, each with its byte span and
/// file. Skips whitespace and `//` comments, but gives a token that starts
/// its line the comment lines directly above it as its doc. Fails at a
/// character no token starts with, or at a string left open.
pub fn lex(src: Str, file: i64) -> Result<List<Tok>, String> {
    let b = src.as_bytes();
    let mut toks: Vec<Tok> = vec![];
    let mut i = 0;
    // The comment lines since the last blank line or code, and whether the
    // current line has had a token, or anything at all.
    let (mut doc, mut coded, mut blank) = (Vec::<String>::new(), false, true);
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c.is_ascii_whitespace() {
            if c == b'\n' {
                if blank {
                    doc.clear();
                }
                (coded, blank) = (false, true);
            }
            i += 1;
            continue;
        } else if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            if !coded {
                let text = &src[start + 2..i];
                doc.push(
                    text.strip_prefix(' ')
                        .unwrap_or(text)
                        .trim_end()
                        .to_string(),
                );
            }
            blank = false;
            continue;
        }
        let n = toks.len();
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            toks.push(tok("ident", &src[start..i], start, i, file));
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
            toks.push(tok(kind, &src[start..i], start, i, file));
        } else if c == b'"' {
            let mut text = String::new();
            i += 1;
            loop {
                match src[i..].chars().next() {
                    None => {
                        return Err(format!(
                            "{i}:{i}: expected \" to close the string, found the end of the file"
                        ));
                    }
                    Some('\n') => {
                        return Err(format!(
                            "{i}:{}: expected \" to close the string, found the end of the line",
                            i + 1
                        ));
                    }
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
                            found => {
                                let (at, end) = (i + 1, i + 1 + found.map_or(0, char::len_utf8));
                                let found = found.map_or("the end of the file".to_string(), |c| {
                                    format!("'{c}'")
                                });
                                return Err(format!(
                                    "{at}:{end}: expected \\, \", n or t after \\, found {found}"
                                ));
                            }
                        });
                        i += 2;
                    }
                    Some(ch) => {
                        text.push(ch);
                        i += ch.len_utf8();
                    }
                }
            }
            toks.push(tok("str", &text, start, i, file));
        } else if let Some(s) = SYMS.iter().find(|s| src[i..].starts_with(**s)) {
            i += s.len();
            toks.push(tok("sym", s, start, i, file));
        } else if "(){}[]<>,;:.=+-*/%!|".contains(c as char) {
            i += 1;
            toks.push(tok("sym", &src[start..i], start, i, file));
        } else {
            let ch = src[i..].chars().next().unwrap();
            return Err(format!(
                "{i}:{}: expected a name, number, string or symbol, found '{ch}'",
                i + ch.len_utf8()
            ));
        }
        // A token starting its line takes the comment lines above it.
        if toks.len() > n {
            if !coded {
                toks[n].doc = doc.join("\n").into();
            }
            doc.clear();
            (coded, blank) = (true, false);
        }
    }
    toks.push(tok("eof", "", b.len(), b.len(), file));
    Ok(toks.into())
}

// ---- Parser steps ----

fn get(toks: &[Tok], pos: i64) -> Tok {
    usize::try_from(pos)
        .ok()
        .and_then(|p| toks.get(p))
        .or(toks.last())
        .cloned()
        .unwrap()
}

pub fn at(toks: List<Tok>, pos: i64) -> Result<Cur, String> {
    let (tok_, prev) = (
        get(&toks, pos),
        if pos > 0 {
            get(&toks, pos - 1)
        } else {
            tok("sof", "", 0, 0, toks.first().map_or(0, |t| t.file))
        },
    );
    Ok(Cur {
        toks,
        pos,
        tok: tok_,
        prev,
    })
}

pub fn at_level(toks: List<Tok>, pos: i64, level: i64) -> Result<AtLevel, String> {
    Ok(AtLevel {
        cur: at(toks, pos)?,
        level,
    })
}

pub fn at_level_value(
    toks: List<Tok>,
    pos: i64,
    level: i64,
    left: Value,
) -> Result<AtLevelValue, String> {
    Ok(AtLevelValue {
        cur: at(toks, pos)?,
        level,
        left,
    })
}

// Loop heads: the token at `pos`, and what the loop has gathered so far.

pub fn at_file(toks: List<Tok>, pos: i64, acc: File) -> Result<AtAccFile, String> {
    Ok(AtAccFile {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_funcs(toks: List<Tok>, pos: i64, acc: List<Func>) -> Result<AtAccFuncList, String> {
    Ok(AtAccFuncList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_impls(toks: List<Tok>, pos: i64, acc: List<ImplRef>) -> Result<AtAccImplRefList, String> {
    Ok(AtAccImplRefList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_body(toks: List<Tok>, pos: i64, acc: BodyItems) -> Result<AtAccBodyItems, String> {
    Ok(AtAccBodyItems {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_fields(toks: List<Tok>, pos: i64, acc: List<Field>) -> Result<AtAccFieldList, String> {
    Ok(AtAccFieldList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_variants(
    toks: List<Tok>,
    pos: i64,
    acc: List<Variant>,
) -> Result<AtAccVariantList, String> {
    Ok(AtAccVariantList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_types(
    toks: List<Tok>,
    pos: i64,
    acc: List<TypeExpr>,
) -> Result<AtAccTypeExprList, String> {
    Ok(AtAccTypeExprList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_segs(toks: List<Tok>, pos: i64, acc: List<Ident>) -> Result<AtAccIdentList, String> {
    Ok(AtAccIdentList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_args(toks: List<Tok>, pos: i64, acc: List<Arg>) -> Result<AtAccArgList, String> {
    Ok(AtAccArgList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_arms(toks: List<Tok>, pos: i64, acc: List<Arm>) -> Result<AtAccArmList, String> {
    Ok(AtAccArmList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_branches(
    toks: List<Tok>,
    pos: i64,
    acc: List<Branch>,
) -> Result<AtAccBranchList, String> {
    Ok(AtAccBranchList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_pats(toks: List<Tok>, pos: i64, acc: List<Pat>) -> Result<AtAccPatList, String> {
    Ok(AtAccPatList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_pat_fields(
    toks: List<Tok>,
    pos: i64,
    acc: List<PatField>,
) -> Result<AtAccPatFieldList, String> {
    Ok(AtAccPatFieldList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_values(toks: List<Tok>, pos: i64, acc: List<Value>) -> Result<AtAccValueList, String> {
    Ok(AtAccValueList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_entries(toks: List<Tok>, pos: i64, acc: List<Entry>) -> Result<AtAccEntryList, String> {
    Ok(AtAccEntryList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_apply_args(
    toks: List<Tok>,
    pos: i64,
    acc: List<ApplyArg>,
) -> Result<AtAccApplyArgList, String> {
    Ok(AtAccApplyArgList {
        cur: at(toks, pos)?,
        acc,
    })
}

pub fn at_value(toks: List<Tok>, pos: i64, left: Value) -> Result<AtValue, String> {
    Ok(AtValue {
        cur: at(toks, pos)?,
        left,
    })
}

fn shown(t: &Tok) -> String {
    match &*t.kind {
        "eof" => "the end of the file".into(),
        "str" => format!("{:?}", &*t.text),
        _ => format!("'{}'", t.text),
    }
}

/// Matches `toks[pos]` and moves past it; the error says what was wanted,
/// prefixed with the token's byte range ("at:end: ").
fn matched(
    toks: List<Tok>,
    pos: i64,
    ok: impl Fn(&Tok) -> bool,
    what: &str,
) -> Result<Cur, String> {
    let t = get(&toks, pos);
    if !ok(&t) {
        return Err(format!(
            "{}:{}: expected {what}, found {}",
            t.at,
            t.end,
            shown(&t)
        ));
    }
    Ok(Cur {
        toks,
        pos: pos + 1,
        tok: t.clone(),
        prev: t,
    })
}

/// Fails at toks[pos], saying what was expected there instead.
pub fn expected_value(toks: List<Tok>, pos: i64, what: Str) -> Result<RValue, String> {
    matched(toks, pos, |_| false, &what).map(|_| unreachable!())
}

/// Fails at toks[pos], saying what was expected there instead.
pub fn expected_target(toks: List<Tok>, pos: i64, what: Str) -> Result<RTarget, String> {
    matched(toks, pos, |_| false, &what).map(|_| unreachable!())
}

/// Matches the symbol or word `text` at toks[pos] and moves past it; fails
/// with "expected <what>, found <token>" otherwise.
pub fn expect(toks: List<Tok>, pos: i64, text: Str, what: Str) -> Result<Cur, String> {
    matched(
        toks,
        pos,
        |t| (&*t.kind == "sym" || &*t.kind == "ident") && t.text == text,
        &what,
    )
}

/// Matches the `>` closing type arguments or parameters; fails on `>=` (which
/// lexes as the comparison) with a hint to write `> =`.
pub fn close_angle(toks: List<Tok>, pos: i64) -> Result<Cur, String> {
    let t = get(&toks, pos);
    if &*t.kind == "sym" && &*t.text == ">=" {
        return Err(format!(
            "{}:{}: write `> =` here: `>=` is the comparison operator",
            t.at, t.end
        ));
    }
    matched(
        toks,
        pos,
        |t| &*t.kind == "sym" && &*t.text == ">",
        "> to close the type arguments",
    )
}

/// Matches an identifier that isn't a keyword at toks[pos] and moves past it;
/// fails, saying what was wanted, otherwise.
pub fn need_ident(toks: List<Tok>, pos: i64, what: Str) -> Result<Cur, String> {
    matched(
        toks,
        pos,
        |t| &*t.kind == "ident" && !KEYWORDS.contains(&&*t.text),
        &what,
    )
}

/// Matches any identifier at toks[pos], keywords included (field and
/// parameter names may be keywords), and moves past it.
pub fn need_name(toks: List<Tok>, pos: i64, what: Str) -> Result<Cur, String> {
    matched(toks, pos, |t| &*t.kind == "ident", &what)
}

/// Matches a string at toks[pos] and moves past it; fails, saying what was
/// wanted, otherwise.
pub fn need_str(toks: List<Tok>, pos: i64, what: Str) -> Result<Cur, String> {
    matched(toks, pos, |t| &*t.kind == "str", &what)
}

pub fn is_sym(t: Tok, s: Str) -> Result<bool, String> {
    Ok(&*t.kind == "sym" && t.text == s)
}

pub fn is_word(t: Tok, s: Str) -> Result<bool, String> {
    Ok(&*t.kind == "ident" && t.text == s)
}

/// Whether t closes a bracket (`)`, `]` or `}`): where a list stops, leaving
/// it for its caller to expect.
pub fn is_close(t: Tok) -> Result<bool, String> {
    Ok(&*t.kind == "sym" && matches!(&*t.text, ")" | "}" | "]"))
}

// ---- Syntax ----

pub fn span_of(t: Tok) -> Result<Span, String> {
    Ok(Span {
        at: t.at,
        end: t.end,
        file: t.file,
    })
}

pub fn mk_ident(t: Tok) -> Result<Ident, String> {
    Ok(Ident {
        name: t.text,
        span: Span {
            at: t.at,
            end: t.end,
            file: t.file,
        },
    })
}

/// Where a path's span ends: the end of its last segment, or of its first if
/// it has no others.
pub fn path_end(t: Tok, segs: List<Ident>) -> Result<i64, String> {
    Ok(segs.last().map_or(t.end, |s| s.span.end))
}

pub fn int_of(t: Tok) -> Result<i64, String> {
    t.text
        .parse()
        .map_err(|_| format!("{}:{}: number {} is out of range", t.at, t.end, t.text))
}

pub fn float_of(t: Tok) -> Result<f64, String> {
    t.text
        .parse()
        .map_err(|_| format!("{}:{}: number {} is out of range", t.at, t.end, t.text))
}

/// Splits a "file:function" string token at its last `:` into the
/// implementation it names, with the token's span; fails unless both sides
/// are there and the function is an identifier.
pub fn impl_of(t: Tok) -> Result<ImplRef, String> {
    let bad = || {
        format!(
            "{}:{}: expected \"file:function\", found {:?}",
            t.at, t.end, &*t.text
        )
    };
    let (file, function) = t.text.rsplit_once(':').ok_or_else(bad)?;
    let is_ident = function
        .chars()
        .enumerate()
        .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()));
    if file.is_empty() || function.is_empty() || !is_ident {
        return Err(bad());
    }
    Ok(ImplRef {
        file: file.into(),
        function: function.into(),
        span: Span {
            at: t.at,
            end: t.end,
            file: t.file,
        },
    })
}

/// Splits a "file:Type" string token into the file and the type it names, at
/// the first `:` after the file's last `/`, with the token's span; fails unless
/// both sides are there and the type is a name, a path (`a::B`, `pkg.T`) or a
/// pointer to one (`*T`).
pub fn type_impl_of(t: Tok) -> Result<ImplRef, String> {
    let bad = || {
        format!(
            "{}:{}: expected \"file:Type\", found {:?}",
            t.at, t.end, &*t.text
        )
    };
    let after = t.text.rfind('/').map_or(0, |i| i + 1);
    let colon = t.text[after..].find(':').ok_or_else(bad)? + after;
    let (file, ty) = (&t.text[..colon], &t.text[colon + 1..]);
    let ident = |s: &str| {
        !s.is_empty()
            && s.chars()
                .enumerate()
                .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
    };
    let path = ty.trim_start_matches('*');
    let segs: Vec<&str> = if path.contains("::") {
        path.split("::").collect()
    } else {
        path.split('.').collect()
    };
    if file.is_empty() || !segs.iter().all(|s| ident(s)) {
        return Err(bad());
    }
    Ok(ImplRef {
        file: file.into(),
        function: ty.into(),
        span: Span {
            at: t.at,
            end: t.end,
            file: t.file,
        },
    })
}

/// Whether t is an operator of precedence `level`: `||` at 0, `&&` at 1,
/// comparisons and `in` at 2, `+ -` at 3, `* / %` at 4.
pub fn op_at(t: Tok, level: i64) -> Result<bool, String> {
    let ops: &[&str] = match level {
        0 => &["||"],
        1 => &["&&"],
        2 => &["==", "!=", "<", "<=", ">", ">="],
        3 => &["+", "-"],
        4 => &["*", "/", "%"],
        _ => &[],
    };
    Ok((&*t.kind == "sym" && ops.contains(&&*t.text))
        || (level == 2 && &*t.kind == "ident" && &*t.text == "in"))
}

/// Builds `l op r` from its operands, spanning both: a Binary, or an In when
/// the operator is `in`.
pub fn combine(op: Tok, l: Value, r: Value) -> Result<Value, String> {
    let span = Span {
        at: l.span.at,
        end: r.span.end,
        file: l.span.file,
    };
    let kind = if &*op.text == "in" {
        ValueKind::In(Arc::new(ValueKindIn {
            key: Arc::new(l),
            map: Arc::new(r),
        }))
    } else {
        ValueKind::Binary(Arc::new(ValueKindBinary {
            op: op.text,
            l: Arc::new(l),
            r: Arc::new(r),
        }))
    };
    Ok(Value {
        kind: Arc::new(kind),
        span,
    })
}

/// Builds `op e` for a prefix `!` or `-`, spanning both; `-` before a number
/// literal folds into a negative literal instead.
pub fn prefix(op: Tok, e: Value) -> Result<Value, String> {
    let span = Span {
        at: op.at,
        end: e.span.end,
        file: op.file,
    };
    let kind = match (&*op.text, &*e.kind) {
        ("-", ValueKind::Int(i)) => ValueKind::Int(-i),
        ("-", ValueKind::Float(f)) => ValueKind::Float(-f),
        _ => ValueKind::Unary(Arc::new(ValueKindUnary {
            op: op.text.clone(),
            e: Arc::new(e),
        })),
    };
    Ok(Value {
        kind: Arc::new(kind),
        span,
    })
}

/// Makes one pattern of alternatives: the pattern itself if there's one, else
/// an Or of them all, spanning from the first to the last.
pub fn or_of(pats: List<Pat>) -> Result<Pat, String> {
    if pats.len() == 1 {
        return Ok(pats[0].clone());
    }
    let span = Span {
        at: pats[0].span.at,
        end: pats.last().unwrap().span.end,
        file: pats[0].span.file,
    };
    Ok(Pat {
        kind: Arc::new(PatKind::Or(pats)),
        span,
    })
}

/// The span covering toks[from..to], a call's arguments as written; empty, at
/// toks[from], if there are none.
pub fn inner_span(toks: List<Tok>, from: i64, to: i64) -> Result<Span, String> {
    let first = get(&toks, from);
    if to <= from {
        return Ok(Span {
            at: first.at,
            end: first.at,
            file: first.file,
        });
    }
    let last = get(&toks, to - 1);
    Ok(Span {
        at: first.at,
        end: last.end,
        file: first.file,
    })
}

/// Checks that a proc's first edge leaves its signature node, the proc's own
/// name, and passes the edges on; fails at that edge, saying what to write,
/// otherwise.
pub fn starts_at_sig(name: Ident, edges: List<Edge>) -> Result<List<Edge>, String> {
    if let Some(e) = edges.first() {
        let src = &e.source;
        if src.segs.len() != 1 || src.segs[0].name != name.name {
            let shown = src
                .segs
                .iter()
                .map(|s| &*s.name)
                .collect::<Vec<_>>()
                .join(".");
            return Err(format!(
                "{}:{}: the body of proc {} starts from {}: write `{} -> ..` first, found {shown}",
                src.span.at, src.span.end, name.name, name.name, name.name
            ));
        }
    }
    Ok(edges)
}
