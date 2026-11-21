use crate::app::types::{self, List, Str};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

/// Returns 0..i-1.
pub fn range(i: i64) -> Result<types::ForEachStartOut, String> {
    Ok(types::ForEachStartOut { l: (0..i).collect(), s: i.to_string().into() })
}

pub fn square(i: i64, _l: List<i64>) -> Result<i64, String> {
    Ok(i * i)
}

pub fn sum(l: List<i64>) -> Result<i64, String> {
    Ok(l.iter().sum())
}

pub fn repeat(a: i64, b: Str) -> Result<Str, String> {
    Ok(b.repeat(a as usize).into())
}

// Grow is stateful so the `this < 10` loop terminates: each pass adds 3.
static GROWN: AtomicI64 = AtomicI64::new(0);
static LOGGED: Mutex<Vec<i64>> = Mutex::new(Vec::new());

pub fn grow(s: Str) -> Result<i64, String> {
    Ok(s.len() as i64 + GROWN.fetch_add(3, Ordering::SeqCst) + 3)
}

pub fn describe(i: i64) -> Result<Str, String> {
    Ok(format!("sum={i}").into())
}

pub fn log(i: i64) -> Result<(), String> {
    LOGGED.lock().unwrap().push(i);
    Ok(())
}

pub fn finish(r: types::Outcome) -> Result<Str, String> {
    let tag = match r {
        types::Outcome::I(_) => "I",
        types::Outcome::S => "S",
        types::Outcome::B(_) => "B",
    };
    Ok(format!("end:{tag}").into())
}

#[cfg(test)]
pub fn reset() {
    GROWN.store(0, Ordering::SeqCst);
    LOGGED.lock().unwrap().clear();
}

#[cfg(test)]
pub fn logged() -> Vec<i64> {
    LOGGED.lock().unwrap().clone()
}

/// Passes Main's command-line arguments on to Bar.
pub fn args(a: i64, b: Str) -> Result<types::MainStartOut, String> {
    Ok(types::MainStartOut { a, b })
}

/// Returns base + 2.
pub fn build(base: i64) -> Result<types::Expr, String> {
    let lit = |i| Arc::new(types::Expr::Lit(i));
    Ok(types::Expr::Add(Arc::new(types::ExprAdd { l: lit(base), r: lit(2) })))
}

fn ev(e: &types::Expr) -> i64 {
    match e {
        types::Expr::Lit(i) => *i,
        types::Expr::Neg(x) => -ev(x),
        types::Expr::Add(a) => ev(&a.l) + ev(&a.r),
    }
}

pub fn eval(e: types::Expr, scale: i64) -> Result<i64, String> {
    Ok(ev(&e) * scale)
}

pub fn total(xs: List<i64>) -> Result<i64, String> {
    Ok(xs.iter().sum())
}

pub fn look(e: types::Expr) -> Result<types::Expr, String> {
    Ok(e)
}

/// Whether |i| < 100.
pub fn small(i: i64) -> Result<bool, String> {
    Ok(i.abs() < 100)
}

/// Gives each name its index as a slot.
pub fn declare(names: List<Str>, query: Str) -> Result<types::ResolveDeclareOut, String> {
    let scope = names
        .into_iter()
        .enumerate()
        .map(|(i, n)| (n, types::ScopeValue { slot: i as i64, kind: "var".into() }))
        .collect();
    Ok(types::ResolveDeclareOut { scope, query })
}

pub fn adjust(scope: types::Scope, query: Str) -> Result<types::ResolveDeclareOut, String> {
    Ok(types::ResolveDeclareOut { scope, query })
}

pub fn found(name: Str, slot: i64, kind: Str, total: i64) -> Result<Str, String> {
    Ok(format!("{name}: slot {slot} ({kind}) of {total}").into())
}

pub fn missing(name: Str, known: List<Str>) -> Result<Str, String> {
    let known: Vec<&str> = known.iter().map(|k| &**k).collect();
    Ok(format!("{name}: not declared; known: {}", known.join(", ")).into())
}
