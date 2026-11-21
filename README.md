# vandorf (Rust)

Proof-of-concept Rust rewrite of the vandorf compiler.

```
cargo run -- check examples/demo/demo.sys
cargo run -- doc   examples/demo/demo.sys -o doc.json           # visualizer document
cargo run -- go    examples/demo/demo.sys -o examples/demo/go/app
cargo run -- rust  examples/demo/demo.sys -o examples/demo/rs/src/app
cargo run -- serve                                              # workbench on :8080

(cd examples/demo/go && go test ./...)
(cd examples/demo/rs && cargo test)
```

Pipeline: `parser` (chumsky, source → `ast`) → `sema` (name resolution,
`this` typing, for-each regions) → `document` (grid layout, same JSON as the Go
`ui/document`) and `codegen::{go, rust}`.

## Workbench

`vandorf serve [--addr 127.0.0.1:8080] [--dir .]` serves an editor with live
compile, the visualizer (in an iframe, or standalone at `/viz`), the generated
Go and Rust, and the implementation signatures each backend expects.

## Typegen and codegen

Each backend emits two artifacts:

- **types**: a named type for every struct/enum shape (`codegen::types::TypeTable`:
  structurally equal types share one name, taken from where the shape first
  appears, e.g. `ForEachStartOut`, `BarEndR`). Depends on nothing.
- **code**: one state machine per proc. Depends on the types and the impls.

| | types | code |
|---|---|---|
| Go | `<out>/types/types_vandorf.go`, its own package (`--types-dir .` to inline) | `<out>/*_vandorf.go` |
| Rust | `<out>/types.rs` | `<out>/mod.rs`, `<out>/procs.rs` |

Go needs the separate package: the code package imports the implementations,
so implementations in other packages can only reach the types if they live
elsewhere.

## Implementations

A func names its implementation per backend; each backend uses the reference
with its file extension:

```
func Start(i: int): { l: []int, s: string } = "impl.go:Range", "../impls.rs:range";
```

Paths are relative to the generated code's directory (`-o`).

- **Go**: same directory → called directly; elsewhere → imported (path from
  `go.mod`, or `--module`).
  `func Range(ctx context.Context, i int) (types.ForEachStartOut, error)`;
  void funcs return just `error`.
- **Rust**: mapped to a module path (`src/impls.rs` → `crate::impls`; the
  output's place under `src/` comes from `Cargo.toml`, or `--module-dir`).
  `pub fn range(i: i64) -> Result<types::ForEachStartOut, E>` with
  `E: Into<Box<dyn Error + Send + Sync>>`; void is `()`.

Funcs without a reference for a backend call a generated stub that returns an
error. `vandorf go`/`rust` warn when a referenced file or function is missing,
printing the expected signature.
