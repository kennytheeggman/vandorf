//! Implementations of vcc's I/O interfaces.

pub fn read(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path}: {e}"))
}
