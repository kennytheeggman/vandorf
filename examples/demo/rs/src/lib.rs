//! Demo of the Rust backend: `app` is generated from ../demo.sys by
//! `vandorf rust ../demo.sys -o src/app`.

pub mod app;
mod effects;
mod impls;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_each() {
        // sum of squares of 0..4
        assert_eq!(app::run_for_each(5).unwrap(), 30);
        assert_eq!(app::run_for_each(0).unwrap(), 0);
    }

    #[test]
    fn bar() {
        impls::reset();
        // Start: "ab"; Loop: 2+3=5 -> loops, 2+6=8 -> loops, 2+9=11 -> ForEach(11)
        // ForEach(11) = 385 -> Branch2Begin "sum=385" -> Foo.Call 7 -> Branch2End -> End(S)
        assert_eq!(&*app::run_bar(1, "ab".into()).unwrap(), "end:S");
        assert_eq!(impls::logged(), vec![7]);
    }

    #[test]
    fn calc() {
        // typedefs, an alias and literals: 5 + 10 + 100
        assert_eq!(app::run_calc(3).unwrap(), 115);
    }

    #[test]
    fn classify() {
        use app::types::{Expr, ExprAdd};
        let lit = |i| Expr::Lit(i);
        let neg = |e| Expr::Neg(std::sync::Arc::new(e));
        let add = |l, r| Expr::Add(std::sync::Arc::new(ExprAdd { l: std::sync::Arc::new(l), r: std::sync::Arc::new(r) }));
        for (e, want) in [
            (lit(4), "small even"),
            (lit(3), "literal"),
            (lit(400), "literal"),
            (neg(lit(1)), "negated literal"),
            (neg(neg(lit(1))), "minus negated literal"),
            (neg(neg(neg(lit(1)))), "minus minus negated literal"),
            (add(lit(1), lit(2)), "sum of literals"),
            (add(lit(1), neg(lit(2))), "something else"),
        ] {
            assert_eq!(&*app::run_classify(e.clone()).unwrap(), want, "{e:?}");
        }
    }

    #[test]
    fn resolve() {
        let names: app::types::List<app::types::Str> = ["a", "b", "tmp"].into_iter().map(Into::into).collect();
        for (q, want) in [
            ("b", "b: slot 1 (var) of 3"),
            ("len", "len: slot 3 (builtin) of 3"),
            ("tmp", "tmp: not declared; known: a, b, len"),
        ] {
            assert_eq!(&*app::run_resolve(names.clone(), q.into()).unwrap(), want);
        }
    }
}
