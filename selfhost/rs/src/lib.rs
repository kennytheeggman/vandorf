//! vcc: the vandorf compiler, written in vandorf. `vcc` (generated from
//! ../sys by `vandorf build`) is the compiler; `leaf` holds its pure leaf
//! functions and `io` the implementations of its I/O interfaces.

pub mod vcc;
mod io;
mod leaf;
