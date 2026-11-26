# Contributing

## Building

With a vcc built (see [Bootstrapping](bootstrapping.md)), from the
repository root:

```
rs/target/debug/vcc build      # regenerate rs/src/vcc and rs/src/bin, then cargo build
rs/target/debug/vcc check      # just check src/
```

`vcc build` runs `cargo build` in `rs/` after generating. The new vcc
replaces the one that built it, so a change that breaks vcc leaves a vcc
that may not build the fix. Keep a copy of a working one
(`cp rs/target/debug/vcc /tmp/vcc-ok`) before changing the compiler, and
build with it if the new one fails.

## Testing

```
cd rs && cargo test
```

| Test | What it checks |
|---|---|
| `parse.rs` | Every `.sys` file in `src/` and the demo parses; the corpus's ASTs; syntax errors and where they point. |
| `sema.rs` | Typedefs, declarations, how types show and accept each other, and each program's graph or errors. |
| `doc.rs` | Diagram layouts. |
| `gen.rs` | Generated Rust and Go: types, procs, servers. |
| `report.rs` | How diagnostics are rendered. |
| `manifest.rs` | Which manifests are accepted, and the errors for the rest. |
| `cli.rs` | The command line on small projects: namespaces, errors naming their file, finding projects, implementation paths. |
| `ide.rs` | The IDE's routes and their guards. |
| `grove.rs` | grove's routes on a repository made for the test: pins and their ideas, branching, merging and rebasing, layouts, discarding, manifests and guards; and the `grove` commands against a running grove. |

Also check the **fixed point** after any change to `src/`: vcc rebuilds
itself identically.

```
cp -R rs/src/vcc /tmp/vcc-gen
rs/target/debug/vcc build
diff -r /tmp/vcc-gen rs/src/vcc && echo "fixed point holds"
```

### Golden files

Most tests compare vcc's output with a file in `rs/tests/golden/<group>/`,
over the programs in `rs/tests/corpus/` (and `examples/demo`). The corpus
files end in `.sys.txt`, so the IDE and the compilers don't take them for
sources: many are wrong on purpose.

When a change is meant to change output, record the new output and review
it:

```
cd rs
VCC_BLESS=1 cargo test         # write vcc's output as the golden files
git diff tests/golden          # every change should be one you meant
```

A mismatch names the group, the case and the first differing line. A test
fails, too, if a program with a golden file stops checking, so cases
can't silently drop out.

To add a test program, add a `.sys.txt` to `rs/tests/corpus/` and bless.

## Conventions

- **Control flow in `.sys`, leaves pure.** A new step that decides what
  happens next belongs in a proc. A leaf (`*_leaf.rs`) takes values and
  returns a value or an error; it does no I/O. Anything that touches the
  outside world goes through an interface in `io.sys` or `ide/ide.sys`.
- **Errors say what was expected and what was found**, and point at the
  span they're about.
- **Comments** say what a proc or type is for, at the top of the file and
  above each declaration, in plain sentences. The one directly above a
  declaration is its doc comment: a leaf's `///` doc must say the same,
  word for word, or `vcc build` fails. A comment about a group of
  declarations takes a blank line after it, so it's no one's doc.
- **Go and Rust behave the same.** A language feature lands in both
  backends, with golden files for both.

## Commits

Commit the `.sys`, the leaves and the golden files together; generated code
isn't checked in. Before pushing: `cargo test` in `rs/`, the fixed point,
and `vcc build -C examples/demo` with both the demo's Go and Rust building.
