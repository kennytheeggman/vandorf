/// Counts the characters of s. Implemented outside the generated
/// package/module.
pub fn call(s: crate::app::types::Str) -> Result<i64, String> {
    Ok(s.len() as i64)
}
