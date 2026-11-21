# vandorf (Rust)

Proof-of-concept Rust rewrite of the vandorf compiler.

```
cargo run -- check examples/demo/demo.sys
cargo run -- doc   examples/demo/demo.sys -o doc.json           # visualizer document
cargo run -- go    examples/demo/demo.sys -o examples/demo/go/app --main
cargo run -- rust  examples/demo/demo.sys -o examples/demo/rs/src/app --main
cargo run -- serve                                              # workbench on :8080

(cd examples/demo/go && go test ./... && go run ./app/main 1 ab)   # end:S
(cd examples/demo/rs && cargo test && cargo run -- 1 ab)          # end:S
```

Pipeline: `parser` (chumsky, source → `ast`) → `sema` (name resolution,
`this` typing, for-each regions) → `document` (grid layout, same JSON as the Go
`ui/document`) and `codegen::{go, rust}`.

## Language additions

```
type Expr = enum { Lit(int), Neg(Expr), Add({ l: Expr, r: Expr }) };

proc Calc(Build): int {
	...
	Build -> Eval(e = Add({ l = Neg(this), r = Lit(10) }), scale = 1);
	Eval -> Eval(e = Build, scale = 2) as Twice;
	Twice -> Total(xs = [Eval, Twice, 100]);
}
```

- **Typedefs**: `type Name = <type>;` at the top level, in any order, and
  possibly recursive (a recursive one must be a struct or enum). A typedef is
  a name for its structure, so it and its definition written out are
  interchangeable. Codegen names the shape after the typedef; typedefs of
  primitives, arrays, or already-named shapes become aliases (`type Ids =
  []int`). Fields that would make a type contain itself are `*T` in Go and
  `Box<T>` in Rust.
- **Aliases**: `-> B(..) as B2` calls a separate instance of `B` named `B2`,
  with its own output and outgoing edge, so one func can be used at several
  points in a proc. Later edges call the same instance with `-> B2(..)`, or
  with `as B2` again.
- **Literals**: `3`, `-1.5`, `"s"`, `true`, `[a, b]`, `{ x = .., y = .. }`
  (every field), `Tag(value)` and `Tag()` for a variant without payload.
  They're checked against the parameter's type (an int literal also passes
  as a float) and can nest references to node outputs.

Field, parameter and argument names may be keywords (`{ type: string }`).

### Switches and expressions

```
Look switch {
	Lit(n) -> switch {
		(Small(i = n) && n % 2 == 0) -> Say(s = "small even"),
		else -> Say(s = "literal")
	},
	Neg(Lit(_)) -> Say(s = "negated literal"),
	Neg(e) -> Classify(e = e) as Inner,
	Add({ l = Lit(_), r = Lit(_) }) -> Say(s = "sum of literals"),
	Add(_) -> Say(s = "something else")
};
```

A switch's arms are tried in order, and the first that matches is taken.
They're either all patterns or all conditions; `else` may come last.

- **Patterns** match structure, in the shape of the literal that builds
  the value: `Tag(p)` (or `Tag()` for a variant without payload),
  `{ f = p, g = p }` listing every field, `_`, a binding `x`, and `p | q`
  (which can't bind yet). A pattern switch needs an `else` unless its arms
  cover every value; missing cases are reported as patterns
  (`doesn't cover Add({ l = Neg(_), r = _ })`), and arms that can't match
  are errors.
- **Conditions** are `(expr)` arms and need an `else`. A pattern arm can lead
  to a nested `switch { (cond) -> .., else -> .. }` that sees its bindings;
  nothing falls through to the outer switch.
- **Expressions** are used for conditions and arguments alike:
  `|| && ! == != < <= > >= + - * / %`, literals, and calls of funcs:
  `F(param = value)`. A call in an expression runs the func's
  implementation directly (it isn't a node). Operands of a binary operator
  have the same primitive type (an int literal also passes as a float);
  `+` concatenates strings. `&&` and `||` short-circuit, calls included.

### Maps

```
type Scope = map[string]{ slot: int, kind: string };
...
Declare -> Adjust(scope = this.scope + ["len": { slot = this.scope.len, kind = "builtin" }] - ["tmp"], ..);
Adjust switch {
	(this.query in this.scope) -> Found(slot = this.scope[this.query].slot, ..),
	else -> Missing(known = this.scope.keys, ..)
};
```

- `map[K]V`, with `int` or `string` keys. Maps are values: passing one on
  never lets anyone else change it.
- Literals `["a": 1, "b": 2]` and `[:]`, where a map type is expected.
- `m[k]` looks a key up and `xs[i]` indexes an array; both fail the proc if
  it isn't there, so check first with `k in m` (or `.len`).
- `m + n` is a new map with both (`n` wins); `m - keys` one without `keys`
  (an array, e.g. `["tmp"]`).
- `.len` of maps, arrays and strings; `.keys` and `.values` of maps, in key
  order, so `for each (this.m.keys)` visits keys in the same order in both
  backends. A `for each` takes any array expression.
- Go: `map[K]V`; Rust: `BTreeMap<K, V>`. Implementations must not change a
  map they're given (Go maps are references).

The diagram draws one edge per call a switch can end in, labelled with the
arm heads on the way (`Lit(n) → else`).

## Workbench

`vandorf serve [--addr 127.0.0.1:8080] [--dir .]` serves an editor with live
compile, the visualizer (in an iframe, or standalone at `/viz`), the generated
Go and Rust, and the implementation signatures each backend expects.

## Typegen and codegen

Each backend emits two artifacts:

- **types**: a named type for every struct/enum shape (`codegen::types::TypeTable`:
  structurally equal types share one name, taken from where the shape first
  appears, e.g. `ForEachStartOut`, `MainStartOut`). Depends on nothing.
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

## Main

`--main` also writes an executable that runs the top-level proc `Main`
(an error if there is none). `Main`'s parameters must be `int`, `float`,
`bool` or `string`; they are taken from the command line in declared order,
and the result is printed (`usage: main <a: int> <b: string>`).

- **Go**: `<dir>/main/main_vandorf.go`, a `package main` importing the code
  package.
- **Rust**: `src/main.rs`, calling `<crate>::<module>::run_main`, so the crate
  needs a `lib.rs` that declares the module.

Neither backend overwrites a file it didn't generate, so an existing
hand-written `src/main.rs` is left alone (the command fails instead). The
workbench shows the main whenever the program has a `Main`.
