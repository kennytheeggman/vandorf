use crate::app::types;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

/// Returns 0..i-1.
pub fn range(i: i64) -> Result<types::ForEachStartOut, String> {
    Ok(types::ForEachStartOut { l: (0..i).collect(), s: i.to_string() })
}

pub fn square(i: i64, _l: Vec<i64>) -> Result<i64, String> {
    Ok(i * i)
}

pub fn sum(l: Vec<i64>) -> Result<i64, String> {
    Ok(l.iter().sum())
}

pub fn repeat(a: i64, b: String) -> Result<String, String> {
    Ok(b.repeat(a as usize))
}

// Grow is stateful so the `this < 10` loop terminates: each pass adds 3.
static GROWN: AtomicI64 = AtomicI64::new(0);
static LOGGED: Mutex<Vec<i64>> = Mutex::new(Vec::new());

pub fn grow(s: String) -> Result<i64, String> {
    Ok(s.len() as i64 + GROWN.fetch_add(3, Ordering::SeqCst) + 3)
}

pub fn describe(i: i64) -> Result<String, String> {
    Ok(format!("sum={i}"))
}

pub fn log(i: i64) -> Result<(), String> {
    LOGGED.lock().unwrap().push(i);
    Ok(())
}

pub fn finish(r: types::Outcome) -> Result<String, String> {
    let tag = match r {
        types::Outcome::I(_) => "I",
        types::Outcome::S => "S",
        types::Outcome::B(_) => "B",
    };
    Ok(format!("end:{tag}"))
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
pub fn args(a: i64, b: String) -> Result<types::MainStartOut, String> {
    Ok(types::MainStartOut { a, b })
}

/// Returns base + 2.
pub fn build(base: i64) -> Result<types::Expr, String> {
    let lit = |i| Box::new(types::Expr::Lit(i));
    Ok(types::Expr::Add(Box::new(types::ExprAdd { l: lit(base), r: lit(2) })))
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

pub fn total(xs: Vec<i64>) -> Result<i64, String> {
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
pub fn declare(names: Vec<String>, query: String) -> Result<types::ResolveDeclareOut, String> {
    let scope = names
        .into_iter()
        .enumerate()
        .map(|(i, n)| (n, types::ScopeValue { slot: i as i64, kind: "var".into() }))
        .collect();
    Ok(types::ResolveDeclareOut { scope, query })
}

pub fn adjust(scope: types::Scope, query: String) -> Result<types::ResolveDeclareOut, String> {
    Ok(types::ResolveDeclareOut { scope, query })
}

pub fn found(name: String, slot: i64, kind: String, total: i64) -> Result<String, String> {
    Ok(format!("{name}: slot {slot} ({kind}) of {total}"))
}

pub fn missing(name: String, known: Vec<String>) -> Result<String, String> {
    Ok(format!("{name}: not declared; known: {}", known.join(", ")))
}
