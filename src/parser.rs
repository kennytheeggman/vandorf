//! Chumsky grammar for `.sys` files.
//!
//! ```text
//! file      := (interface | func ';' | proc ';'?)*
//! interface := 'interface' IDENT '{' (func ';')* '}'
//! func      := 'func' IDENT '(' (field (',' field)*)? ')' (':' type)? ('=' STRING)?
//! proc      := 'proc' IDENT '(' IDENT ')' ':' type '{' ((func | proc | edge) ';')* '}'
//! type      := '[' ']' type | 'enum' '{' variant,* '}' | '{' field,* '}' | IDENT
//! variant   := IDENT ('(' type ')')?
//! field     := IDENT ':' type
//! edge      := path '->' target
//!            | path 'switch' '{' ('(' STRING ')' '->' target),* '}'
//!            | path 'collect' 'each' '(' IDENT ')' '->' target
//!            | path 'for' 'each' '(' path ')' '->' target
//! target    := path '(' (arg (',' arg)*)? ')' ('as' IDENT)?
//! arg       := IDENT '=' (IDENT '(' path ')' | path)
//! path      := IDENT ('.' IDENT)*
//! ```
//!
//! The `= "file:function"` suffix on a func names the code that implements it.
//! `//` starts a line comment.

use crate::ast::*;
use chumsky::prelude::*;

pub type ParseError<'a> = Rich<'a, char>;
type Extra<'a> = extra::Err<ParseError<'a>>;

const KEYWORDS: &[&str] = &[
    "interface", "func", "proc", "enum", "switch", "collect", "for", "each", "as",
];

pub fn parse(src: &str) -> Result<File, Vec<ParseError<'_>>> {
    file().parse(src).into_result()
}

/// Whitespace and `//` comments.
fn ws<'a>() -> impl Parser<'a, &'a str, (), Extra<'a>> + Clone {
    let comment = just("//").then(any().and_is(just('\n').not()).repeated()).ignored();
    any()
        .filter(|c: &char| c.is_whitespace())
        .ignored()
        .or(comment)
        .repeated()
}

/// A punctuation token followed by trailing whitespace.
fn sym<'a>(s: &'static str) -> impl Parser<'a, &'a str, (), Extra<'a>> + Clone {
    just(s).ignored().then_ignore(ws())
}

fn kw<'a>(s: &'static str) -> impl Parser<'a, &'a str, (), Extra<'a>> + Clone {
    text::ascii::keyword(s).ignored().then_ignore(ws())
}

fn ident<'a>() -> impl Parser<'a, &'a str, Ident, Extra<'a>> + Clone {
    text::ascii::ident()
        .try_map(|s: &str, span| {
            if KEYWORDS.contains(&s) {
                Err(Rich::custom(span, format!("'{s}' is a keyword")))
            } else {
                Ok(Ident { name: s.to_string(), span })
            }
        })
        .then_ignore(ws())
        .labelled("identifier")
}

fn path<'a>() -> impl Parser<'a, &'a str, Path, Extra<'a>> + Clone {
    ident()
        .separated_by(sym("."))
        .at_least(1)
        .collect::<Vec<_>>()
        .map_with(|segs, e| Path { segs, span: e.span() })
        .labelled("path")
}

fn string<'a>() -> impl Parser<'a, &'a str, (String, Span), Extra<'a>> + Clone {
    let escape = just('\\').ignore_then(choice((
        just('\\'),
        just('"'),
        just('n').to('\n'),
        just('t').to('\t'),
    )));
    none_of("\\\"\n")
        .or(escape)
        .repeated()
        .collect::<String>()
        .delimited_by(just('"'), just('"'))
        .map_with(|s, e| (s, e.span()))
        .then_ignore(ws())
        .labelled("string")
}

fn type_expr<'a>() -> impl Parser<'a, &'a str, TypeExpr, Extra<'a>> + Clone {
    recursive(|ty| {
        let field = ident()
            .then_ignore(sym(":"))
            .then(ty.clone())
            .map(|(name, ty)| Field { name, ty });

        let array = sym("[")
            .ignore_then(sym("]"))
            .ignore_then(ty.clone())
            .map(|t| TypeExprKind::Array(Box::new(t)));

        let variant = ident()
            .then(ty.clone().delimited_by(sym("("), sym(")")).or_not())
            .map(|(name, payload)| Variant { name, payload });
        let enum_ = kw("enum").ignore_then(
            variant
                .separated_by(sym(","))
                .allow_trailing()
                .at_least(1)
                .collect()
                .delimited_by(sym("{"), sym("}"))
                .map(TypeExprKind::Enum),
        );

        let struct_ = field
            .separated_by(sym(","))
            .allow_trailing()
            .at_least(1)
            .collect()
            .delimited_by(sym("{"), sym("}"))
            .map(TypeExprKind::Struct);

        let named = ident().map(TypeExprKind::Named);

        choice((array, enum_, struct_, named))
            .map_with(|kind, e| TypeExpr { kind, span: e.span() })
            .labelled("type")
    })
}

fn field<'a>() -> impl Parser<'a, &'a str, Field, Extra<'a>> + Clone {
    ident()
        .then_ignore(sym(":"))
        .then(type_expr())
        .map(|(name, ty)| Field { name, ty })
}

fn impl_ref<'a>() -> impl Parser<'a, &'a str, ImplRef, Extra<'a>> + Clone {
    string().try_map(|(s, span), _| {
        let bad = || Rich::custom(span, format!("expected \"file:function\", found \"{s}\""));
        let (file, function) = s.rsplit_once(':').ok_or_else(bad)?;
        let is_ident = function
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()));
        if file.is_empty() || function.is_empty() || !is_ident {
            return Err(bad());
        }
        Ok(ImplRef { file: file.to_string(), function: function.to_string(), span })
    })
}

fn func<'a>() -> impl Parser<'a, &'a str, Func, Extra<'a>> + Clone {
    kw("func")
        .ignore_then(ident())
        .then(
            field()
                .separated_by(sym(","))
                .collect()
                .delimited_by(sym("("), sym(")")),
        )
        .then(sym(":").ignore_then(type_expr()).or_not())
        .then(sym("=").ignore_then(impl_ref()).or_not())
        .map(|(((name, params), ret), imp)| Func { name, params, ret, imp })
}

fn target<'a>() -> impl Parser<'a, &'a str, Target, Extra<'a>> + Clone {
    let value = ident()
        .then(path().delimited_by(sym("("), sym(")")))
        .map(|(tag, payload)| ArgValue::Variant { tag, payload })
        .or(path().map(ArgValue::Ref));
    let arg = ident()
        .then_ignore(sym("="))
        .then(value)
        .map(|(name, value)| Arg { name, value });
    path()
        .then(arg.separated_by(sym(",")).collect().delimited_by(sym("("), sym(")")))
        .then(kw("as").ignore_then(ident()).or_not())
        .map(|((node, args), alias)| Target { node, args, alias })
}

fn edge<'a>() -> impl Parser<'a, &'a str, Edge, Extra<'a>> + Clone {
    let arrow = || sym("->");
    let call = arrow().ignore_then(target()).map(EdgeKind::Call);
    let arm = string()
        .delimited_by(sym("("), sym(")"))
        .then_ignore(arrow())
        .then(target())
        .map(|((cond, cond_span), target)| Arm { cond, cond_span, target });
    let match_ = kw("switch").ignore_then(
        arm.separated_by(sym(","))
            .allow_trailing()
            .at_least(1)
            .collect()
            .delimited_by(sym("{"), sym("}"))
            .map(EdgeKind::Match),
    );
    let collect = kw("collect")
        .ignore_then(kw("each"))
        .ignore_then(ident().delimited_by(sym("("), sym(")")))
        .then_ignore(arrow())
        .then(target())
        .map(|(dim, target)| EdgeKind::Collect { dim, target });
    let foreach = kw("for")
        .ignore_then(kw("each"))
        .ignore_then(path().delimited_by(sym("("), sym(")")))
        .then_ignore(arrow())
        .then(target())
        .map(|(dim, target)| EdgeKind::ForEach { dim, target });

    path()
        .then(choice((call, match_, collect, foreach)))
        .map_with(|(source, kind), e| Edge { source, kind, span: e.span() })
}

fn node<'a>() -> impl Parser<'a, &'a str, Node, Extra<'a>> + Clone {
    recursive(|node| {
        enum Item {
            Node(Node),
            Edge(Edge),
        }
        let item = node.map(Item::Node).or(edge().map(Item::Edge));
        let proc = kw("proc")
            .ignore_then(ident())
            .then(ident().delimited_by(sym("("), sym(")")))
            .then_ignore(sym(":"))
            .then(type_expr())
            .then(
                item.then_ignore(sym(";"))
                    .repeated()
                    .collect::<Vec<_>>()
                    .delimited_by(sym("{"), sym("}")),
            )
            .map(|(((name, entry), ret), items)| {
                let mut p = Proc { name, entry, ret, nodes: vec![], edges: vec![] };
                for it in items {
                    match it {
                        Item::Node(n) => p.nodes.push(n),
                        Item::Edge(e) => p.edges.push(e),
                    }
                }
                Node::Proc(p)
            });
        func().map(Node::Func).or(proc)
    })
}

fn file<'a>() -> impl Parser<'a, &'a str, File, Extra<'a>> {
    enum Item {
        Interface(Interface),
        Node(Node),
    }
    let interface = kw("interface")
        .ignore_then(ident())
        .then(
            func()
                .then_ignore(sym(";"))
                .repeated()
                .collect()
                .delimited_by(sym("{"), sym("}")),
        )
        .map(|(name, funcs)| Interface { name, funcs });
    // Top-level funcs need a ';'; it's optional after a proc's closing brace.
    let item = choice((
        interface.map(Item::Interface),
        func().then_ignore(sym(";")).map(|f| Item::Node(Node::Func(f))),
        node().then_ignore(sym(";").or_not()).map(Item::Node),
    ));
    ws().ignore_then(item.repeated().collect::<Vec<_>>())
        .then_ignore(end())
        .map(|items| {
            let mut f = File::default();
            for it in items {
                match it {
                    Item::Interface(i) => f.interfaces.push(i),
                    Item::Node(n) => f.nodes.push(n),
                }
            }
            f
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_impl_ref() {
        let f = parse(r#"func A(x: int): int = "impl/a.go:DoA";"#).unwrap();
        let Node::Func(func) = &f.nodes[0] else { panic!() };
        let imp = func.imp.as_ref().unwrap();
        assert_eq!((imp.file.as_str(), imp.function.as_str()), ("impl/a.go", "DoA"));
    }

    #[test]
    fn rejects_bad_impl_ref() {
        assert!(parse(r#"func A(x: int) = "nocolon";"#).is_err());
        assert!(parse(r#"func A(x: int) = "a.go:1x";"#).is_err());
    }

    #[test]
    fn keywords_are_not_idents() {
        assert!(parse("func for(x: int);").is_err());
    }

    #[test]
    fn parses_edges() {
        let src = r#"
            proc P(A): int {
                func A(): { l: []int };   // comment
                func B(i: int): int;
                func C(l: []int): int;
                A for each (A.l) -> B(i = this);
                B collect each (B) -> C(l = this);
            }
        "#;
        let f = parse(src).unwrap();
        let Node::Proc(p) = &f.nodes[0] else { panic!() };
        assert_eq!(p.nodes.len(), 3);
        assert!(matches!(p.edges[0].kind, EdgeKind::ForEach { .. }));
        assert!(matches!(p.edges[1].kind, EdgeKind::Collect { .. }));
    }
}
