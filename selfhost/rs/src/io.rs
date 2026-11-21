//! Implementations of vcc's I/O interfaces.

use crate::vcc::types::Str;

pub fn read(path: Str) -> Result<Str, String> {
    std::fs::read_to_string(&*path).map(Str::from).map_err(|e| format!("cannot read {path}: {e}"))
}
