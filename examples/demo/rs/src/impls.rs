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

pub fn finish(r: types::BarEndR) -> Result<String, String> {
    let tag = match r {
        types::BarEndR::I(_) => "I",
        types::BarEndR::S => "S",
        types::BarEndR::B(_) => "B",
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
