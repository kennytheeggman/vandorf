//! Chumsky grammar for `.sys` files.
//!
//! ```text
//! file      := (typedef | interface | func ';' | proc ';'?)*
//! typedef   := 'type' IDENT '=' type ';'
//! interface := 'interface' IDENT '{' (func ';')* '}'
//! func      := 'func' IDENT '(' (field (',' field)*)? ')' (':' type)? ('=' STRING (',' STRING)*)?
//! proc      := 'proc' IDENT '(' IDENT ')' ':' type '{' ((func | proc | edge) ';')* '}'
//! type      := '[' ']' type | 'enum' '{' variant,* '}' | '{' field,* '}' | IDENT
//! variant   := IDENT ('(' type ')')?
//! field     := IDENT ':' type
//! edge      := path '->' target
//!            | path 'switch' '{' ('(' STRING ')' '->' target),* '}'
//!            | path 'collect' 'each' '(' IDENT ')' '->' target
//!            | path 'for' 'each' '(' path ')' '->' target
//! target    := path '(' (arg (',' arg)*)? ')' ('as' IDENT)?
//! arg       := NAME '=' value
//! value     := INT | FLOAT | STRING | 'true' | 'false'
//!            | IDENT '(' value? ')'               enum variant
//!            | '{' (arg (',' arg)*)? '}'          struct
//!            | '[' (value (',' value)*)? ']'      array
//!            | path
//! path      := IDENT ('.' NAME)*
//! ```
//!
//! NAME is any identifier, keywords included (fields can be called `type`);
//! IDENT excludes keywords.
//!
//! The `= "file:function", ...` suffix on a func names the code that implements
//! it, one reference per target language (picked by file extension).
//! `//` starts a line comment.

use crate::ast::*;
use chumsky::prelude::*;

pub type ParseError<'a> = Rich<'a, char>;
type Extra<'a> = extra::Err<ParseError<'a>>;

const KEYWORDS: &[&str] = &[
    "interface", "func", "proc", "enum", "switch", "collect", "for", "each", "as", "type",
    "true", "false",
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

/// Any identifier, keywords included: field, parameter and argument names.
fn name<'a>() -> impl Parser<'a, &'a str, Ident, Extra<'a>> + Clone {
    text::ascii::ident()
        .map_with(|s: &str, e| Ident { name: s.to_string(), span: e.span() })
        .then_ignore(ws())
        .labelled("name")
}

fn path<'a>() -> impl Parser<'a, &'a str, Path, Extra<'a>> + Clone {
    ident()
        .then(sym(".").ignore_then(name()).repeated().collect::<Vec<_>>())
        .map_with(|(first, rest), e| {
            let mut segs = vec![first];
            segs.extend(rest);
            Path { segs, span: e.span() }
        })
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
        let field = name()
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
    name()
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
        .then(
            sym("=")
                .ignore_then(impl_ref().separated_by(sym(",")).at_least(1).collect::<Vec<_>>())
                .or_not(),
        )
        .map(|(((name, params), ret), imps)| Func { name, params, ret, imps: imps.unwrap_or_default() })
}

fn number<'a>() -> impl Parser<'a, &'a str, ValueKind, Extra<'a>> + Clone {
    just('-')
        .or_not()
        .then(text::int(10))
        .then(just('.').then(text::digits(10)).or_not())
        .to_slice()
        .try_map(|s: &str, span| {
            let v = if s.contains('.') {
                s.parse().map(ValueKind::Float).ok()
            } else {
                s.parse().map(ValueKind::Int).ok()
            };
            v.ok_or_else(|| Rich::custom(span, format!("number {s} is out of range")))
        })
        .then_ignore(ws())
        .labelled("number")
}

fn value<'a>() -> impl Parser<'a, &'a str, Value, Extra<'a>> + Clone {
    recursive(|value| {
        let arg = name()
            .then_ignore(sym("="))
            .then(value.clone())
            .map(|(name, value)| Arg { name, value });
        let variant = ident()
            .then(value.clone().or_not().delimited_by(sym("("), sym(")")))
            .map(|(tag, payload): (Ident, Option<Value>)| ValueKind::Variant { tag, payload: payload.map(Box::new) });
        let struct_ = arg
            .separated_by(sym(","))
            .allow_trailing()
            .collect()
            .delimited_by(sym("{"), sym("}"))
            .map(ValueKind::Struct);
        let array = value
            .separated_by(sym(","))
            .allow_trailing()
            .collect()
            .delimited_by(sym("["), sym("]"))
            .map(ValueKind::Array);
        choice((
            number(),
            string().map(|(s, _)| ValueKind::Str(s)),
            kw("true").to(ValueKind::Bool(true)),
            kw("false").to(ValueKind::Bool(false)),
            struct_,
            array,
            variant,
            path().map(ValueKind::Ref),
        ))
        .map_with(|kind, e| Value { kind, span: e.span() })
        .labelled("value")
    })
}

fn target<'a>() -> impl Parser<'a, &'a str, Target, Extra<'a>> + Clone {
    let arg = name()
        .then_ignore(sym("="))
        .then(value())
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
        Type(TypeDecl),
        Interface(Interface),
        Node(Node),
    }
    let typedef = kw("type")
        .ignore_then(ident())
        .then_ignore(sym("="))
        .then(type_expr())
        .then_ignore(sym(";"))
        .map(|(name, ty)| TypeDecl { name, ty });
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
        typedef.map(Item::Type),
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
                    Item::Type(t) => f.types.push(t),
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
        let f = parse(r#"func A(x: int): int = "impl/a.go:DoA", "a.rs:do_a";"#).unwrap();
        let Node::Func(func) = &f.nodes[0] else { panic!() };
        let got: Vec<_> = func.imps.iter().map(|i| (i.file.as_str(), i.function.as_str())).collect();
        assert_eq!(got, vec![("impl/a.go", "DoA"), ("a.rs", "do_a")]);
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

    #[test]
    fn parses_typedefs_and_keyword_fields() {
        let f = parse("type T = enum { Leaf(int), Node({ type: string, kids: []T }) };").unwrap();
        assert_eq!(f.types[0].name.name, "T");
        assert!(parse("type for = int;").is_err());
    }

    #[test]
    fn parses_literals() {
        let src = r#"
            proc P(A): int {
                func A(): int;
                func B(v: int): int;
                A -> B(i = -3, f = 1.5, s = "a\"b", t = true, l = [1, 2,], e = S(), v = T({ x = this, type = [] }), r = A.x.type) as B2;
            }
        "#;
        let f = parse(src).unwrap();
        let Node::Proc(p) = &f.nodes[0] else { panic!() };
        let EdgeKind::Call(t) = &p.edges[0].kind else { panic!() };
        assert_eq!(t.alias.as_ref().unwrap().name, "B2");
        let kinds: Vec<String> = t.args.iter().map(|a| format!("{:?}", a.value.kind)).collect();
        assert!(kinds[0].starts_with("Int(-3)") && kinds[1].starts_with("Float(1.5)"), "{kinds:?}");
        assert!(kinds[2].contains(r#"Str("a\"b")"#) && kinds[3] == "Bool(true)", "{kinds:?}");
        assert!(matches!(&t.args[4].value.kind, ValueKind::Array(v) if v.len() == 2));
        assert!(matches!(&t.args[5].value.kind, ValueKind::Variant { payload: None, .. }));
        assert!(matches!(&t.args[6].value.kind, ValueKind::Variant { payload: Some(p), .. } if matches!(p.kind, ValueKind::Struct(_))));
        assert!(matches!(&t.args[7].value.kind, ValueKind::Ref(p) if p.to_string() == "A.x.type"));
    }
}
