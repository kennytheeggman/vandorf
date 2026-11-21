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
        assert_eq!(app::run_bar(1, "ab".into()).unwrap(), "end:S");
        assert_eq!(impls::logged(), vec![7]);
    }
}
