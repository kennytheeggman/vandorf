# vandorf (Rust)

Proof-of-concept Rust rewrite of the vandorf compiler.

```
cargo run -- check examples/demo/demo.sys
cargo run -- doc   examples/demo/demo.sys -o doc.json      # visualizer document
cargo run -- go    examples/demo/demo.sys -o examples/demo/app
cd examples/demo && go test ./...
```

Pipeline: `parser` (chumsky, source → `ast`) → `sema` (name resolution,
`this` typing, for-each regions) → `document` (grid layout, same JSON as the Go
`ui/document`) and `codegen::go`.

## Implementations

A func may name the code that implements it:

```
func Start(i: int): { l: []int, s: string } = "impl.go:Range";
func Call(s: string): int = "../effects/foo.go:Call";
```

For Go, the path is relative to the generated package directory. Same
directory: called directly. Other directory: imported (path derived from
`go.mod`, or `--module`). The function takes `ctx` and the params positionally
and returns `(ret, error)`, or just `error` for void funcs:

```go
func Range(ctx context.Context, i int) (struct{ L []int; S string }, error)
```

Types are structural, so they map onto anonymous Go types (struct fields in
declaration order, enums as `struct{ Tag string; <Variant> <payload>... }`)
and impls never need generated types. Funcs without an implementation call a
generated stub in `stubs_vandorf.go` that returns an error. `vandorf go` warns
when a referenced file or function is missing and prints the expected signature.
