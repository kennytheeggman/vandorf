//! Chumsky grammar for `.sys` files.
//!
//! ```text
//! file      := (typedef | interface | func ';' | proc ';'?)*
//! typedef   := 'type' IDENT ('<' IDENT,+ '>')? '=' type ';'
//! interface := 'interface' IDENT '{' (func ';')* '}'
//! func      := 'func' IDENT '(' (field (',' field)*)? ')' (':' type)? ('=' STRING (',' STRING)*)?
//! proc      := 'proc' IDENT '(' IDENT ')' ':' type '{' ((func | proc | edge) ';')* '}'
//! type      := '[' ']' type | 'map' '[' type ']' type | 'enum' '{' variant,* '}' | '{' field,* '}'
//!            | IDENT ('<' type,+ '>')?               a typedef, or a generic one with arguments
//! variant   := IDENT ('(' type ')')?
//! field     := IDENT ':' type
//! edge      := path '->' target
//!            | path 'switch' '{' arm,* '}'
//!            | path 'collect' 'each' '(' IDENT ')' '->' target
//!            | path 'for' 'each' '(' expr ')' '->' target
//! target    := path '(' (arg (',' arg)*)? ')' ('as' IDENT)?
//!            | 'id' '(' expr ')'                   end the proc with a value
//! arm       := (pat | '(' expr ')' | 'else') '->' (target | 'switch' '{' arm,* '}')
//! pat       := alt ('|' alt)*
//! alt       := '_' | IDENT                          wildcard, binding
//!            | IDENT '(' pat? ')'                   enum variant
//!            | '{' (NAME '=' pat),* '}'             struct, every field
//! arg       := NAME '=' expr
//! expr      := and ('||' and)*
//! and       := cmp ('&&' cmp)*
//! cmp       := sum (('==' | '!=' | '<' | '<=' | '>' | '>=' | 'in') sum)?
//! sum       := term (('+' | '-') term)*
//! term      := unary (('*' | '/' | '%') unary)*
//! unary     := ('!' | '-') unary | postfix
//! postfix   := atom ('[' expr ']' | '.' NAME)*         lookup, field
//! atom      := INT | FLOAT | STRING | 'true' | 'false'
//!            | path '(' ((NAME '=')? expr),* ')'   enum variant or func call
//!            | '{' (arg (',' arg)*)? '}'          struct
//!            | '{' '..' expr (',' arg)* '}'      struct update: base's other fields
//!            | '[' (expr (',' expr)*)? ']'        array
//!            | '[' ':' ']' | '[' (expr ':' expr),* ']'   map
//!            | path | '(' expr ')'
//! path      := IDENT ('.' NAME)*
//! ```
//!
//! NAME is any identifier, keywords included (fields can be called `type`);
//! IDENT excludes keywords.
//!
//! The `= "file:function", ...` suffix on a func names the code that implements
//! it, one reference per target language (picked by file extension). The file
//! is relative to the `.sys` file.
//! `//` starts a line comment.

use crate::ast::*;
use chumsky::prelude::*;

/// Source text whose spans carry the file they're in.
type In<'a> = chumsky::input::WithContext<Span, &'a str>;

pub type ParseError<'a> = Rich<'a, char, Span>;
type Extra<'a> = extra::Err<ParseError<'a>>;

const KEYWORDS: &[&str] = &[
    "interface", "func", "proc", "enum", "switch", "collect", "for", "each", "as", "type",
    "true", "false", "else", "map", "in", "id",
];

/// Parses one source file; its spans carry `id`.
pub fn parse(src: &str, id: FileId) -> Result<File, Vec<ParseError<'_>>> {
    file().parse(src.with_context(id)).into_result()
}

/// Whitespace and `//` comments.
fn ws<'a>() -> impl Parser<'a, In<'a>, (), Extra<'a>> + Clone {
    let comment = just("//").then(any().and_is(just('\n').not()).repeated()).ignored();
    any()
        .filter(|c: &char| c.is_whitespace())
        .ignored()
        .or(comment)
        .repeated()
}

/// A punctuation token followed by trailing whitespace.
fn sym<'a>(s: &'static str) -> impl Parser<'a, In<'a>, (), Extra<'a>> + Clone {
    just(s).ignored().then_ignore(ws())
}

/// The `>` closing type arguments or parameters. `>=` right after it is
/// rejected: it reads as the comparison operator.
fn close_angle<'a>() -> impl Parser<'a, In<'a>, (), Extra<'a>> + Clone {
    just('>')
        .then(just('=').rewind().or_not())
        .validate(|(_, eq), e, emitter| {
            if eq.is_some() {
                emitter.emit(Rich::custom(e.span(), "write `> =` here: `>=` is the comparison operator"));
            }
        })
        .ignored()
        .then_ignore(ws())
}

fn kw<'a>(s: &'static str) -> impl Parser<'a, In<'a>, (), Extra<'a>> + Clone {
    text::ascii::keyword(s).ignored().then_ignore(ws())
}

fn ident<'a>() -> impl Parser<'a, In<'a>, Ident, Extra<'a>> + Clone {
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
fn name<'a>() -> impl Parser<'a, In<'a>, Ident, Extra<'a>> + Clone {
    text::ascii::ident()
        .map_with(|s: &str, e| Ident { name: s.to_string(), span: e.span() })
        .then_ignore(ws())
        .labelled("name")
}

fn path<'a>() -> impl Parser<'a, In<'a>, Path, Extra<'a>> + Clone {
    ident()
        .then(sym(".").ignore_then(name()).repeated().collect::<Vec<_>>())
        .map_with(|(first, rest), e| {
            let mut segs = vec![first];
            segs.extend(rest);
            Path { segs, span: e.span() }
        })
        .labelled("path")
}

fn string<'a>() -> impl Parser<'a, In<'a>, (String, Span), Extra<'a>> + Clone {
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

fn type_expr<'a>() -> impl Parser<'a, In<'a>, TypeExpr, Extra<'a>> + Clone {
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

        let map = kw("map")
            .ignore_then(ty.clone().delimited_by(sym("["), sym("]")))
            .then(ty.clone())
            .map(|(k, v)| TypeExprKind::Map(Box::new(k), Box::new(v)));

        let named = ident()
            .then(ty.clone().separated_by(sym(",")).at_least(1).collect::<Vec<_>>().delimited_by(sym("<"), close_angle()).or_not())
            .map(|(n, args)| match args {
                None => TypeExprKind::Named(n),
                Some(args) => TypeExprKind::Inst(n, args),
            });

        choice((array, map, enum_, struct_, named))
            .map_with(|kind, e| TypeExpr { kind, span: e.span() })
            .labelled("type")
    })
}

fn field<'a>() -> impl Parser<'a, In<'a>, Field, Extra<'a>> + Clone {
    name()
        .then_ignore(sym(":"))
        .then(type_expr())
        .map(|(name, ty)| Field { name, ty })
}

fn impl_ref<'a>() -> impl Parser<'a, In<'a>, ImplRef, Extra<'a>> + Clone {
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
        Ok(ImplRef { written: file.to_string(), file: file.to_string(), function: function.to_string(), span })
    })
}

fn func<'a>() -> impl Parser<'a, In<'a>, Func, Extra<'a>> + Clone {
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

fn number<'a>() -> impl Parser<'a, In<'a>, ValueKind, Extra<'a>> + Clone {
    text::int(10)
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

/// `=`, but not the start of `==`.
fn assign<'a>() -> impl Parser<'a, In<'a>, (), Extra<'a>> + Clone {
    just('=').and_is(just("==").not()).ignored().then_ignore(ws())
}

/// An operator token that isn't the start of a longer one.
fn op<'a>(s: &'static str, not_before: &'static [&'static str]) -> impl Parser<'a, In<'a>, (), Extra<'a>> + Clone {
    let longer = choice(not_before.iter().map(|t| just(*t).ignored().boxed()).collect::<Vec<_>>());
    just(s).and_is(longer.not()).ignored().then_ignore(ws())
}

type Expr<'a> = chumsky::Boxed<'a, 'a, In<'a>, Value, Extra<'a>>;

/// One left-associative precedence level.
fn level<'a>(operand: Expr<'a>, ops: Vec<(&'static str, &'static [&'static str], BinOp)>) -> Expr<'a> {
    let ops = choice(ops.into_iter().map(|(s, nb, o)| op(s, nb).to(o).boxed()).collect::<Vec<_>>());
    operand
        .clone()
        .foldl_with(ops.then(operand).repeated(), |l, (o, r), e| Value {
            kind: ValueKind::Binary(o, Box::new(l), Box::new(r)),
            span: e.span(),
        })
        .boxed()
}

fn value<'a>() -> impl Parser<'a, In<'a>, Value, Extra<'a>> + Clone {
    recursive(|expr| {
        let arg = name()
            .then_ignore(assign())
            .then(expr.clone())
            .map(|(name, value)| Arg { name, value });
        let apply_arg = name()
            .then_ignore(assign())
            .then(expr.clone())
            .map(|(n, value)| ApplyArg { name: Some(n), value })
            .or(expr.clone().map(|value| ApplyArg { name: None, value }));
        let apply = path()
            .then(apply_arg.separated_by(sym(",")).allow_trailing().collect().delimited_by(sym("("), sym(")")))
            .map(|(name, args)| ValueKind::Apply { name, args });
        let update = sym("..")
            .ignore_then(expr.clone())
            .then(sym(",").ignore_then(arg.clone().separated_by(sym(",")).allow_trailing().collect::<Vec<_>>()).or_not())
            .delimited_by(sym("{"), sym("}"))
            .map(|(base, args)| ValueKind::Update { base: Box::new(base), args: args.unwrap_or_default() });
        let struct_ = arg
            .separated_by(sym(","))
            .allow_trailing()
            .collect()
            .delimited_by(sym("{"), sym("}"))
            .map(ValueKind::Struct);
        let map = sym("[")
            .then(sym(":"))
            .then(sym("]"))
            .to(ValueKind::Map(vec![]))
            .or(expr
                .clone()
                .then_ignore(sym(":"))
                .then(expr.clone())
                .separated_by(sym(","))
                .allow_trailing()
                .at_least(1)
                .collect()
                .delimited_by(sym("["), sym("]"))
                .map(ValueKind::Map));
        let array = expr
            .clone()
            .separated_by(sym(","))
            .allow_trailing()
            .collect()
            .delimited_by(sym("["), sym("]"))
            .map(ValueKind::Array);
        let atom = choice((
            number(),
            string().map(|(s, _)| ValueKind::Str(s)),
            kw("true").to(ValueKind::Bool(true)),
            kw("false").to(ValueKind::Bool(false)),
            update,
            struct_,
            map,
            array,
            apply,
            path().map(ValueKind::Ref),
        ))
        .map_with(|kind, e| Value { kind, span: e.span() })
        .or(expr.clone().delimited_by(sym("("), sym(")")))
        .labelled("expression");
        enum Post {
            Index(Value),
            Field(Ident),
        }
        let postfix = atom.foldl_with(
            choice((
                expr.clone().delimited_by(sym("["), sym("]")).map(Post::Index),
                sym(".").ignore_then(name()).map(Post::Field),
            ))
            .repeated(),
            |v, p, e| {
                let kind = match p {
                    Post::Index(k) => ValueKind::Index(Box::new(v), Box::new(k)),
                    Post::Field(f) => ValueKind::Field(Box::new(v), f),
                };
                Value { kind, span: e.span() }
            },
        );

        let unary = recursive(|unary| {
            choice((op("!", &["!="]).to(UnOp::Not), op("-", &["->"]).to(UnOp::Neg)))
                .then(unary)
                .map_with(|(o, v): (UnOp, Value), e| {
                    // `-3` is a literal, so it can be passed as a float too.
                    let kind = match (o, v.kind) {
                        (UnOp::Neg, ValueKind::Int(i)) => ValueKind::Int(-i),
                        (UnOp::Neg, ValueKind::Float(f)) => ValueKind::Float(-f),
                        (o, kind) => ValueKind::Unary(o, Box::new(Value { kind, span: v.span })),
                    };
                    Value { kind, span: e.span() }
                })
                .or(postfix)
        });
        let term = level(unary.boxed(), vec![("*", &[], BinOp::Mul), ("/", &["//"], BinOp::Div), ("%", &[], BinOp::Rem)]);
        let sum = level(term, vec![("+", &[], BinOp::Add), ("-", &["->"], BinOp::Sub)]);
        // `None` is `in`.
        let cmp_op = choice((
            op("==", &[]).to(Some(BinOp::Eq)),
            op("!=", &[]).to(Some(BinOp::Ne)),
            op("<=", &[]).to(Some(BinOp::Le)),
            op(">=", &[]).to(Some(BinOp::Ge)),
            op("<", &[]).to(Some(BinOp::Lt)),
            op(">", &[]).to(Some(BinOp::Gt)),
            kw("in").to(None),
        ));
        let cmp = sum
            .clone()
            .then(cmp_op.then(sum).or_not())
            .map_with(|(l, r), e| {
                let kind = match r {
                    Some((Some(o), r)) => ValueKind::Binary(o, Box::new(l), Box::new(r)),
                    Some((None, r)) => ValueKind::In(Box::new(l), Box::new(r)),
                    None => return l,
                };
                Value { kind, span: e.span() }
            })
            .boxed();
        let and = level(cmp, vec![("&&", &[], BinOp::And)]);
        level(and, vec![("||", &[], BinOp::Or)])
    })
}

fn target<'a>() -> impl Parser<'a, In<'a>, Target, Extra<'a>> + Clone {
    let arg = name()
        .then_ignore(assign())
        .then(value())
        .map(|(name, value)| Arg { name, value });
    let id = text::ascii::keyword("id")
        .map_with(|_, e| Path { segs: vec![Ident { name: "id".into(), span: e.span() }], span: e.span() })
        .then_ignore(ws())
        .then(value().delimited_by(sym("("), sym(")")))
        .then(kw("as").ignore_then(ident()).or_not())
        .map(|((node, v), alias)| Target { node, args: vec![], alias, id: Some(v) });
    id.or(path()
        .then(arg.separated_by(sym(",")).collect().delimited_by(sym("("), sym(")")))
        .then(kw("as").ignore_then(ident()).or_not())
        .map(|((node, args), alias)| Target { node, args, alias, id: None }))
}

fn pattern<'a>() -> impl Parser<'a, In<'a>, Pat, Extra<'a>> + Clone {
    recursive(|pat| {
        let variant = ident()
            .then(pat.clone().or_not().delimited_by(sym("("), sym(")")))
            .map(|(tag, payload): (Ident, Option<Pat>)| PatKind::Variant { tag, payload: payload.map(Box::new) });
        let struct_ = name()
            .then_ignore(assign())
            .then(pat.clone())
            .separated_by(sym(","))
            .allow_trailing()
            .collect()
            .delimited_by(sym("{"), sym("}"))
            .map(PatKind::Struct);
        let bind = ident().map(|i| if i.name == "_" { PatKind::Wild } else { PatKind::Bind(i) });
        let alt = choice((variant, struct_, bind)).map_with(|kind, e| Pat { kind, span: e.span() });
        alt.separated_by(op("|", &["||"]))
            .at_least(1)
            .collect::<Vec<_>>()
            .map_with(|mut alts, e| {
                if alts.len() == 1 { alts.pop().unwrap() } else { Pat { kind: PatKind::Or(alts), span: e.span() } }
            })
            .labelled("pattern")
    })
}

/// Switch arms; an arm's body is a call or a nested switch.
fn arms<'a>() -> impl Parser<'a, In<'a>, Vec<Arm>, Extra<'a>> + Clone {
    recursive(|arms| {
        let head = choice((
            kw("else").to(Head::Else),
            just('(').rewind().ignore_then(value()).map(Head::Cond),
            pattern().map(Head::Pat),
        ));
        let body = kw("switch")
            .ignore_then(arms.delimited_by(sym("{"), sym("}")))
            .map_with(|a, e| Body::Switch(a, e.span()))
            .or(target().map(Body::Call));
        head.map_with(|h, e| (h, e.slice().trim().to_string(), e.span()))
            .then_ignore(sym("->"))
            .then(body)
            .map(|((head, label, span), body)| Arm { head, label, span, body })
            .separated_by(sym(","))
            .allow_trailing()
            .at_least(1)
            .collect()
    })
}

fn edge<'a>() -> impl Parser<'a, In<'a>, Edge, Extra<'a>> + Clone {
    let arrow = || sym("->");
    let call = arrow().ignore_then(target()).map(EdgeKind::Call);
    let match_ = kw("switch").ignore_then(arms().delimited_by(sym("{"), sym("}")).map(EdgeKind::Match));
    let collect = kw("collect")
        .ignore_then(kw("each"))
        .ignore_then(ident().delimited_by(sym("("), sym(")")))
        .then_ignore(arrow())
        .then(target())
        .map(|(dim, target)| EdgeKind::Collect { dim, target });
    let foreach = kw("for")
        .ignore_then(kw("each"))
        .ignore_then(value().delimited_by(sym("("), sym(")")))
        .then_ignore(arrow())
        .then(target())
        .map(|(dim, target)| EdgeKind::ForEach { dim, target });

    path()
        .then(choice((call, match_, collect, foreach)))
        .map_with(|(source, kind), e| Edge { source, kind, span: e.span() })
}

fn node<'a>() -> impl Parser<'a, In<'a>, Node, Extra<'a>> + Clone {
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

fn file<'a>() -> impl Parser<'a, In<'a>, File, Extra<'a>> {
    enum Item {
        Type(TypeDecl),
        Interface(Interface),
        Node(Node),
    }
    let params = ident().separated_by(sym(",")).at_least(1).collect::<Vec<_>>().delimited_by(sym("<"), close_angle());
    let typedef = kw("type")
        .ignore_then(ident())
        .then(params.or_not())
        .then_ignore(sym("="))
        .then(type_expr())
        .then_ignore(sym(";"))
        .map(|((name, params), ty)| TypeDecl { name, params: params.unwrap_or_default(), ty });
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

    fn parse0(src: &str) -> Result<File, Vec<ParseError<'_>>> {
        parse(src, 0)
    }

    #[test]
    fn parses_impl_ref() {
        let f = parse0(r#"func A(x: int): int = "impl/a.go:DoA", "a.rs:do_a";"#).unwrap();
        let Node::Func(func) = &f.nodes[0] else { panic!() };
        let got: Vec<_> = func.imps.iter().map(|i| (i.file.as_str(), i.function.as_str())).collect();
        assert_eq!(got, vec![("impl/a.go", "DoA"), ("a.rs", "do_a")]);
    }

    #[test]
    fn rejects_bad_impl_ref() {
        assert!(parse0(r#"func A(x: int) = "nocolon";"#).is_err());
        assert!(parse0(r#"func A(x: int) = "a.go:1x";"#).is_err());
    }

    #[test]
    fn keywords_are_not_idents() {
        assert!(parse0("func for(x: int);").is_err());
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
        let f = parse0(src).unwrap();
        let Node::Proc(p) = &f.nodes[0] else { panic!() };
        assert_eq!(p.nodes.len(), 3);
        assert!(matches!(p.edges[0].kind, EdgeKind::ForEach { .. }));
        assert!(matches!(p.edges[1].kind, EdgeKind::Collect { .. }));
    }

    #[test]
    fn parses_typedefs_and_keyword_fields() {
        let f = parse0("type T = enum { Leaf(int), Node({ type: string, kids: []T }) };").unwrap();
        assert_eq!(f.types[0].name.name, "T");
        assert!(parse0("type for = int;").is_err());
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
        let f = parse0(src).unwrap();
        let Node::Proc(p) = &f.nodes[0] else { panic!() };
        let EdgeKind::Call(t) = &p.edges[0].kind else { panic!() };
        assert_eq!(t.alias.as_ref().unwrap().name, "B2");
        let kinds: Vec<String> = t.args.iter().map(|a| format!("{:?}", a.value.kind)).collect();
        assert!(kinds[0].starts_with("Int(-3)") && kinds[1].starts_with("Float(1.5)"), "{kinds:?}");
        assert!(kinds[2].contains(r#"Str("a\"b")"#) && kinds[3] == "Bool(true)", "{kinds:?}");
        assert!(matches!(&t.args[4].value.kind, ValueKind::Array(v) if v.len() == 2));
        assert!(matches!(&t.args[5].value.kind, ValueKind::Apply { args, .. } if args.is_empty()));
        assert!(matches!(&t.args[6].value.kind, ValueKind::Apply { args, .. } if matches!(args[0].value.kind, ValueKind::Struct(_))));
        assert!(matches!(&t.args[7].value.kind, ValueKind::Ref(p) if p.to_string() == "A.x.type"));
    }

    /// Renders an expression with explicit parentheses, to check precedence.
    fn show(v: &Value) -> String {
        match &v.kind {
            ValueKind::Ref(p) => p.to_string(),
            ValueKind::Int(i) => i.to_string(),
            ValueKind::Float(f) => f.to_string(),
            ValueKind::Str(s) => format!("{s:?}"),
            ValueKind::Bool(b) => b.to_string(),
            ValueKind::Apply { name, args } => {
                let a: Vec<String> = args.iter().map(|a| match &a.name {
                    Some(n) => format!("{} = {}", n.name, show(&a.value)),
                    None => show(&a.value),
                }).collect();
                format!("{name}({})", a.join(", "))
            }
            ValueKind::Struct(_) | ValueKind::Update { .. } => "{..}".into(),
            ValueKind::Array(_) => "[..]".into(),
            ValueKind::Unary(o, e) => format!("({}{})", if *o == UnOp::Not { "!" } else { "-" }, show(e)),
            ValueKind::Binary(o, l, r) => format!("({} {} {})", show(l), o.symbol(), show(r)),
            ValueKind::Map(kvs) => {
                let kvs: Vec<String> = kvs.iter().map(|(k, v)| format!("{}: {}", show(k), show(v))).collect();
                if kvs.is_empty() { "[:]".into() } else { format!("[{}]", kvs.join(", ")) }
            }
            ValueKind::Index(m, k) => format!("{}[{}]", show(m), show(k)),
            ValueKind::Field(v, f) => format!("{}.{}", show(v), f.name),
            ValueKind::In(k, m) => format!("({} in {})", show(k), show(m)),
        }
    }

    fn show_pat(p: &Pat) -> String {
        match &p.kind {
            PatKind::Wild => "_".into(),
            PatKind::Bind(i) => i.name.clone(),
            PatKind::Variant { tag, payload } => format!("{}({})", tag.name, payload.as_deref().map(show_pat).unwrap_or_default()),
            PatKind::Struct(fs) => {
                let fs: Vec<String> = fs.iter().map(|(n, p)| format!("{} = {}", n.name, show_pat(p))).collect();
                format!("{{ {} }}", fs.join(", "))
            }
            PatKind::Or(alts) => alts.iter().map(show_pat).collect::<Vec<_>>().join(" | "),
        }
    }

    fn show_arms(arms: &[Arm]) -> Vec<String> {
        arms.iter().map(|a| {
            let head = match &a.head {
                Head::Else => "else".to_string(),
                Head::Cond(c) => show(c),
                Head::Pat(p) => show_pat(p),
            };
            match &a.body {
                Body::Call(t) => format!("{head} -> {}", t.node),
                Body::Switch(inner, _) => format!("{head} -> switch [{}]", show_arms(inner).join("; ")),
            }
        }).collect()
    }

    fn arms(src: &str) -> Vec<String> {
        let f = parse0(src).unwrap_or_else(|e| panic!("{e:?}"));
        let Node::Proc(p) = &f.nodes[0] else { panic!() };
        let EdgeKind::Match(arms) = &p.edges[0].kind else { panic!() };
        show_arms(arms)
    }

    #[test]
    fn parses_expressions_with_precedence() {
        let got = arms(r#"proc P(A): int { A switch {
            (a || b && !c == d) -> X(),
            (1 + 2 * -x % 3 - y >= F(n = this.l, m = 2) + 1) -> X(),
            ((p - 1) * 2 < 3) -> X(),
            else -> X()
        }; }"#);
        assert_eq!(got, vec![
            "(a || (b && ((!c) == d))) -> X",
            "(((1 + ((2 * (-x)) % 3)) - y) >= (F(n = this.l, m = 2) + 1)) -> X",
            "(((p - 1) * 2) < 3) -> X",
            "else -> X",
        ]);
    }

    #[test]
    fn parses_patterns_and_nested_switches() {
        let got = arms(r#"proc P(A): int { A switch {
            Lit(n) -> switch { (n > 0) -> X(), else -> Y() },
            Neg(Lit(_)) | Add({ l = _, r = Neg(e) }) -> X(),
            Unit() -> X(),
            other -> Y()
        }; }"#);
        assert_eq!(got, vec![
            "Lit(n) -> switch [(n > 0) -> X; else -> Y]",
            "Neg(Lit(_)) | Add({ l = _, r = Neg(e) }) -> X",
            "Unit() -> X",
            "other -> Y",
        ]);
    }

    #[test]
    fn parses_maps() {
        let got = arms(r#"proc P(A): int { A switch {
            (k in this.m && this.m[k].x + 1 > [:].len) -> X(m = this.m + ["a": 1, k: F(i = 2)] - [k]),
            else -> X()
        }; }"#);
        assert_eq!(got[0], r#"((k in this.m) && ((this.m[k].x + 1) > [:].len)) -> X"#);
        assert!(parse0("proc P(A): int { func A(m: map[string][]int): map[int]{ x: int }; }").is_ok());
    }

    #[test]
    fn slash_is_not_a_comment() {
        let got = arms("proc P(A): int { A switch { (x / 2 > 0) -> X(), // comment\n else -> X() }; }");
        assert_eq!(got[0], "((x / 2) > 0) -> X");
    }

    #[test]
    fn minus_is_not_an_arrow() {
        let got = arms("proc P(A): int { A switch { (x-1 > 0) -> X(i = this-1), else -> X() }; }");
        assert_eq!(got[0], "((x - 1) > 0) -> X");
    }
}
