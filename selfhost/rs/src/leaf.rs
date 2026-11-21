//! vcc's leaf funcs. Each is pure: its result depends only on its
//! arguments, and it does no I/O.

use crate::vcc::types::Tok;

pub fn same(s: String) -> Result<String, String> {
    Ok(s)
}

pub fn count(n: i64) -> Result<String, String> {
    Ok(format!("{n} tokens"))
}

/// Multi-character symbols, longest first.
const SYMS: &[&str] = &["->", "==", "!=", "<=", ">=", "&&", "||"];

/// Splits `src` into tokens, ending with an "eof" token. `//` starts a
/// comment.
pub fn lex(src: String) -> Result<Vec<Tok>, String> {
    let b = src.as_bytes();
    let mut toks = vec![];
    let mut i = 0;
    let tok = |kind: &str, text: String, at: usize, end: usize| Tok { kind: kind.into(), text, at: at as i64, end: end as i64 };
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
            toks.push(tok("ident", src[start..i].into(), start, i));
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
            toks.push(tok(kind, src[start..i].into(), start, i));
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
                        let esc = src[i + 1..].chars().next();
                        text.push(match esc {
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
            toks.push(tok("str", text, start, i));
        } else if let Some(s) = SYMS.iter().find(|s| src[i..].starts_with(**s)) {
            i += s.len();
            toks.push(tok("sym", s.to_string(), start, i));
        } else if "(){}[]<>,;:.=+-*/%!|_".contains(c as char) {
            i += 1;
            toks.push(tok("sym", (c as char).to_string(), start, i));
        } else {
            let ch = src[i..].chars().next().unwrap();
            return Err(format!("{i}: unexpected character {ch:?}"));
        }
    }
    toks.push(tok("eof", String::new(), b.len(), b.len()));
    Ok(toks)
}
