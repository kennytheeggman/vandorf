# vandorf (Rust)

Proof-of-concept Rust rewrite of the vandorf compiler.

```
cd examples/demo
cargo run -- check                   # the project in vandorf.toml
cargo run -- build                   # both backends and both bins (or: build calc);
                                     # generated code isn't checked in, so this comes first
cargo run -- doc > doc.json          # visualizer document
cargo run -- serve                   # IDE on :8080 (explorer, diagram/text tabs, terminal)

(cd go && go test ./... && go run ./cmd/demo 1 ab && go run ./cmd/calc 3)        # end:S, 115
(cd rs && cargo test && cargo run --bin demo -- 1 ab && cargo run --bin calc -- 3)  # end:S, 115
```

Every command works on a project (a `vandorf.toml`, below). `-C <dir>`,
anywhere in a command, runs it as if started in `<dir>`:
`vandorf check -C examples/demo`.

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
- **Generic typedefs**: `type R<T> = { x: T, pos: int };`, used with type
  arguments wherever a type goes: `func F(): R<[]Tok>`. Each distinct use is
  its own typedef, the body with the arguments substituted, so `R<int>` and
  `{ x: int, pos: int }` are the same type. Codegen names instances after
  their arguments (`R<[]Tok>` -> `RTokList`, `Map<string, int>` ->
  `MapStringInt`), in Go and Rust alike. Arguments are typedefs, primitives
  (not void), arrays or maps; a struct or enum argument needs a typedef.
  A recursive generic may only use itself with arguments that don't grow
  (`List<T>` inside `List<T>`). Write `> =`, not `>=`, before an `=`.
  Funcs and procs aren't generic.
- **Aliases**: `-> B(..) as B2` calls a separate instance of `B` named `B2`,
  with its own output and outgoing edge, so one func can be used at several
  points in a proc. Later edges call the same instance with `-> B2(..)`, or
  with `as B2` again.
- **Literals**: `3`, `-1.5`, `"s"`, `true`, `[a, b]`, `{ x = .., y = .. }`
  (every field), `Tag(value)` and `Tag()` for a variant without payload.
  They're checked against the parameter's type (an int literal also passes
  as a float) and can nest references to node outputs.

- **Struct updates**: `{ ..base, f = v }` is a struct like `base` with
  field `f` set: `Step(s = { ..this, i = this.i + 1 })`. Every field of
  the expected type is given or read from `base` (it may add fields `base`
  doesn't have, but not drop any); with no expected type, the update has
  `base`'s type. `base` must be a path (a node, `this`, a binding, or a
  field of one), so each field read can be moved separately.

- **`id(value)`** ends a proc with a value, checked against the proc's
  return type: `A switch { (this > 0) -> id({ n = this, tags = [] }), .. }`.
  It's the identity func, built in, so procs don't need an identity leaf per
  result type. Each `id(..)` is its own node (drawn as `id`).

Field, parameter and argument names may be keywords (`{ type: string }`).

An edge may only read outputs of nodes that ran on every path to it (the
node dominates the edge's source in the proc's graph; inside a `for each`,
an outer node must have run before the `for each` opened). Anything else is
a compile error, so generated code never reads a node that hasn't run.

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
- `xs + ys` is a new array with both (`acc + [x]` to append).
- `.len` of maps, arrays and strings; `.keys` and `.values` of maps, in key
  order, so `for each (this.m.keys)` visits keys in the same order in both
  backends. A `for each` takes any array expression.
- Go: `map[K]V`; Rust: `BTreeMap<K, V>`. Implementations must not change a
  map they're given (Go maps are references).

The diagram draws one edge per call a switch can end in, labelled with the
arm heads on the way (`Lit(n) → else`).

## Projects

A `vandorf.toml` (found in the current directory or a parent) names the
source directories and what to build; all its `.sys` files share one
namespace, so any file can use any other's types, interfaces and procs.

```toml
[project]
name = "demo"
sources = ["sys"]          # directories searched for .sys files; default ["."]

[go]                       # optional: the Go library
out = "go/app"             # code package; types in out/types
# package, module (import path of out; default from go.mod), types_dir

[rust]                     # optional: the Rust library
out = "rs/src/app"         # a module under the crate's src/
# module_dir (out relative to src/; default from Cargo.toml)
post = ["cargo build --manifest-path rs/Cargo.toml"]   # after generating Rust
test = ["cargo test --manifest-path rs/Cargo.toml"]    # run by vandorf test

[bin.demo]                 # an executable per backend, running a proc
entry = "Main"             # default "Main"
# go = "go/cmd/demo"       # Go main package; default <go.out>/../cmd/<name>
# rust = "bin/demo.rs"     # under the crate's src/; default bin/<name>.rs
```

`vandorf build` generates the libraries and every bin; `vandorf build calc`
the libraries and that bin. After generating a language it runs that
language's `post` commands ([go] takes them too). `vandorf test` builds
everything, then runs each language's `test` commands. Commands are shell
commands run in order in the manifest's directory; the first that fails fails
the command. Errors name the file they're in. The workbench
compiles a project file together with the rest of its project.

## Workbench

`vandorf serve [--addr 127.0.0.1:8080]` serves an IDE on the working
directory: an explorer, diagram and text tabs with live compile, the
visualizer (also standalone at `/viz`), the generated Go and Rust and the
implementation signatures each backend expects, and terminals, which are
offered only when it listens on a loopback address.

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
func Start(i: int): { l: []int, s: string } = "../go/app/impl.go:Range", "../rs/src/impls.rs:range";
```

Paths are relative to the `.sys` file naming them; each backend resolves them
against its output directory.

- **Go**: same directory → called directly; elsewhere → imported (path from
  `go.mod`, or `--module`).
  `func Range(ctx context.Context, i int) (types.ForEachStartOut, error)`;
  void funcs return just `error`.
- **Rust**: mapped to a module path (`src/impls.rs` → `crate::impls`; the
  output's place under `src/` comes from `Cargo.toml`, or `--module-dir`).
  A file outside the crate's `src/` is mounted in the generated `mod.rs`
  (`#[path = "../../leaves/io.rs"] mod io;`) and called as `super::io::f`;
  its stem must be unique among such files.
  `pub fn range(i: i64) -> Result<types::ForEachStartOut, E>` with
  `E: Into<Box<dyn Error + Send + Sync>>`; void is `()`.

Funcs without a reference for a backend call a generated stub that returns an
error. `vandorf build` warns when a referenced file or function is missing,
naming where the func is declared and printing the expected signature.

## HTTP servers

```toml
[http.api]                   # rs/src/bin/api.rs; Rust only, for now
addr = "127.0.0.1:8099"
routes = { "POST /api/measure" = "Measure", "GET /api/hello" = "Hello" }
static = { "/" = "web/index.html" }
```

Each route runs a top-level proc through its typed `run_<proc>`; only the
request and the result cross as JSON. A proc's parameters come from a JSON
object body, or from the query string (each value read as JSON if it
parses, else as a string). Structs are objects, maps objects keyed by
string, and enums `"Tag"` or `{"Tag": payload}`. Results are 200 and JSON;
a request that doesn't decode is a 400, a proc that fails a 500, both with
`{"error": ".."}`. Static files are embedded in the executable.

`vandorf build` writes `http.rs` into the module (the JSON codec, std-only
server and a `handle_<name>` per server) and the server's executable, which
listens on `addr` unless given `--addr host:port`. The demo's `[http.api]`
serves `sys/api.sys`.

## Bins

A bin (`[bin.<name>]`) runs a top-level proc, `Main` by default (an error if there is
none). Its parameters are either `int`, `float`, `bool` or `string`, taken
from the command line in declared order (`usage: demo <a: int> <b:
string>`), or a single `[]string`, which gets the arguments as they are
(`usage: vcc <argv...>`). The result is printed.

- **Go**: a `package main` importing the code package (`<go.out>/../cmd/<name>`).
- **Rust**: a file under the crate's `src/` (`bin/<name>.rs`) calling
  `<crate>::<module>::run_<entry>`, so the crate needs a `lib.rs` that
  declares the module.

Neither backend overwrites a file it didn't generate, so an existing
hand-written `src/main.rs` is left alone (the command fails instead).
