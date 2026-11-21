pub fn call(s: crate::app::types::Str) -> Result<i64, String> {
    Ok(s.len() as i64)
}
