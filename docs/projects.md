# Projects

A project is a `vandorf.toml` and the `.sys` files under its source
directories. All of them share one namespace, so any file can use any
other's types, interfaces and procs.

## Commands

```
vcc check                # check the project
vcc build [<bin>]        # generate the libraries, and every bin or just <bin>
vcc test                 # build, then run each language's test commands
vcc doc                  # the visualizer document (JSON), on stdout
vcc --help
```

Each works on the nearest `vandorf.toml` in the current directory or its
parents. `-C <dir>`, anywhere in a command, runs it as if started in `<dir>`:
`vcc check -C examples/demo`. Errors name the file and position they're in,
with the line and a caret under it.

After generating a language, `build` runs that language's `post` commands;
`test` builds everything, then runs each language's `test` commands. They're
shell commands run in order in the manifest's directory; the first that
fails fails the command.

## vandorf.toml

```toml
[project]
name = "demo"
sources = ["sys"]          # directories searched for .sys files; default ["."]

[go]                       # optional: the Go library
out = "go/app"             # the code package; types in out/types
# package = "app"          # default: out's directory name
# module = "example.com/demo/go/app"   # out's import path; default from go.mod
# types_dir = "types"      # the types package, relative to out; "." inlines it
post = ["go build -C ./go ./cmd/demo"]
# test = ["go test -C ./go ./..."]

[rust]                     # optional: the Rust library
out = "rs/src/app"         # a module under a crate's src/
# module_dir = "app"       # out relative to src/; default from the crate's Cargo.toml
post = ["cargo build --manifest-path rs/Cargo.toml"]
test = ["cargo test --manifest-path rs/Cargo.toml"]

[python]                   # optional: Python 3.10+ package
out = "py/app"
post = ["python3 -m compileall -q py/app"]
test = ["python3 -m unittest discover -s py"]

[bin.demo]                 # an executable per backend, running a proc
entry = "Main"             # default "Main"
# go = "go/cmd/demo"       # Go main package; default <go.out>/../cmd/<name>
# rust = "bin/demo.rs"     # under the crate's src/; default bin/<name>.rs
# python = "py/demo.py"    # relative to the manifest; default <python.out>/../<name>.py

[http.api]                 # an HTTP server per backend
addr = "127.0.0.1:8099"
# setup = "Setup"
# cli = "Cli"              # `api <command> ...` runs proc Cli instead of serving
routes = { "POST /api/measure" = "Measure", "GET /api/hello" = "Hello" }
# raw = { "GET /health" = "Health" }
static = { "/" = "web/index.html" }
# go = "go/cmd/api"
# rust = "bin/api.rs"
# python = "py/api.py"
```

Unknown fields and tables are errors, so a typo doesn't go unnoticed.
Hidden directories, `target` and `node_modules` aren't searched for
sources.

## Generated code

Each backend emits two artifacts:

- **types**: a named type for every struct and enum shape. Structurally equal
  types share one name, taken from their typedef or from where the shape
  first appears (e.g. `ForEachStartOut`, `MainStartOut`). Depends on
  nothing.
- **code**: one state machine per proc. Depends on the types and the
  implementations.

| | types | code |
|---|---|---|
| Go | `<out>/types/types_vandorf.go`, its own package (`types_dir = "."` to inline) | `<out>/*_vandorf.go` |
| Rust | `<out>/types.rs` | `<out>/mod.rs`, `<out>/procs.rs` |
| Python | `<out>/types.py` | `<out>/__init__.py`, `<out>/procs.py`, and support modules |

Go needs the separate package: the code package imports the
implementations, so implementations in other packages can only reach the
types if they live elsewhere.

Each top-level proc gets a typed entry point. Rust and Python use
`run_<proc>`, and Go uses `Run<Proc>`. Run `vcc build` before using a
generated artifact. The compiler only overwrites files it generated.

## Implementations

A func names its implementation per backend; each backend uses the one with
its file extension:

```
func Start(i: int): { l: []int, s: string } = "../go/app/impl.go:Range", "../rs/src/impls.rs:range", "../py/impls.py:range_values";
```

Paths are relative to the `.sys` file naming them; each backend resolves
them against its output directory.

- **Go**: a file in the code package is called directly; one elsewhere is
  imported (its path from `go.mod`, or `module`).
  `func Range(ctx context.Context, i int) (types.ForEachStartOut, error)`;
  void funcs return just `error`.
- **Rust**: a file in the crate is mapped to a module path (`src/impls.rs` →
  `crate::impls`; the output's place under `src/` comes from `Cargo.toml`,
  or `module_dir`). A file outside the crate's `src/` is mounted in the
  generated `mod.rs` (`#[path = "../../leaves/io.rs"] mod io;`) and called
  as `super::io::f`; its stem must be unique among such files.
  `pub fn range(i: i64) -> Result<types::ForEachStartOut, E>` with
  `E: Into<Box<dyn Error + Send + Sync>>`; void is `()`.
- **Python** imports the named function from its `.py` file. For
  `out = "py/app"`, a sibling `py/impls.py` can use
  `from app import types`. The expected callable is
  `def range_values(i: int) -> types.ForEachStartOut`. Void returns
  `None`. Raise an exception to fail the proc. Callers see
  `app.VandorfError` with the node name and original exception.
  Implementations and procs are synchronous. Spawns and for-each regions
  use threads.

`vcc build` warns when a referenced file or function is missing, naming
where the func is declared and printing the expected signature, after the
func's doc comment. Until it exists, the func calls a generated stub that
fails. With a Python interpreter on `PATH`, the Python backend also
checks that declared functions accept the required positional arguments.
Generated calls validate their input and result values. Type hints are
optional.

Go and Rust implementations are checked against their declarations before
their `post` commands:

- **Its signature**: the parameters' names and types, and the result, as
  above. Types compare by the names codegen gives them, without module or
  package paths, so a generic typedef's instance is its generated name
  (`func Read(path: string): Res<string>` is
  `pub fn read(path: Str) -> Result<ResString, String>`, `R<[]Tok>` is
  `RTokList`), a struct equal to an instance is that instance (`R<int>` and
  `{ x: int, pos: int }` are both `RInt`), and a typedef alias may be
  written either way (`Ids` or `List<i64>`). A parameter may be `_x` (Rust)
  or `_` (both) for any name, and Go's `a, b int` names two.
- **Its doc comment**, `///` lines in Rust and `//` lines in Go above the
  function (and above any attributes): the same text as the declaration's
  [doc comment](language.md#doc-comments), apart from the markers. A doc on
  one side only is a mismatch too.

```
error: Read is implemented by fn read in rs/src/impls.rs, whose signature doesn't match: it returns `Result<Str, _>`, expected `Result<ResString, _>`
 --> sys/io.sys:8:6
  |
8 | func Read(path: string): Res<string> = "../go/app/impl.go:Read", "../rs/src/impls.rs:read";
  |      ^^^^
  = expected:
    /// Reads the file at path.
    pub fn read(path: crate::app::types::Str) -> Result<crate::app::types::ResString, E>
  = found: fn read(path: Str) -> Result<Str, String>
```

Each mismatch is an error, and the build stops with all of a language's.
So a func's declaration, its signature and its doc, is a complete spec of
its implementation: whoever writes it can work from the stub alone, and
vcc rejects one that drifts from it.

## Executables

A bin (`[bin.<name>]`) runs a top-level proc, `Main` by default (an error if
there is none). Its parameters are either `int`, `float`, `bool` or
`string`, taken from the command line in declared order (`usage: demo <a:
int> <b: string>`), or a single `[]string`, which gets the arguments as they
are. The result is printed.

- **Go**: a `package main` importing the code package
  (`<go.out>/../cmd/<name>`).
- **Rust**: a file under the crate's `src/` (`bin/<name>.rs`) calling
  `<crate>::<module>::run_<entry>`, so the crate needs a `lib.rs` that
  declares the module.
- **Python** generates a script next to the package by default. Run it
  with `python3 py/demo.py 2 hello`. The same applies to servers, for
  example `python3 py/api.py --addr 127.0.0.1:8099`. Servers use the Python
  standard library.

## HTTP servers

`[http.<name>]` generates a server for each configured backend, with the
same routes, JSON and responses. Each listens on `addr` unless given
`--addr host:port`.

**Routes** (`routes`) run a top-level proc through its typed entry point;
only the request and the result cross as JSON.

- A proc's parameters come from a JSON object body, or from the query string
  (each value read as JSON if it parses, else as a string).
- Structs are objects, maps objects keyed by string, and enums `"Tag"` or
  `{"Tag": payload}`.
- Results are 200 and JSON. A request that doesn't decode is a 400, a proc
  that fails a 500, both with `{"error": ".."}`.

**Raw routes** (`raw`) hand the proc the request and send back what it
returns, for anything that isn't JSON in and out. The proc takes one
parameter shaped `{ method: string, path: string, query: map[string]string,
headers: map[string]string, body: string }` and returns `{ status: int,
headers: map[string]string, body: string }`. vcc's own IDE is served this
way.

**Static files** (`static`) map a URL path to a file, relative to the
manifest, embedded in the executable.

**Setup**: a server with `setup` runs that top-level proc, which takes no
parameters, once before it listens; if it fails, the server exits without
listening. Its result is the server's context: every route and raw proc must
take a parameter `ctx` of exactly its type, which the server fills (a
request's `ctx` is ignored). The context is read-only; state that changes
between requests belongs behind an interface.

**Commands**: a server with `cli` is a command line too. When its first
argument is a command rather than a flag (or is `-h` or `--help`), it runs
that top-level proc on the arguments instead of serving. The proc takes a
single `[]string` and gives nothing (`void`); it fails to exit 1 with its
error. With no arguments, or `--addr`, the server serves as usual. grove
uses this for `grove branch` and the other commands it gives agents.

- **Go**: `http_vandorf.go` in the code package (the JSON codec, a
  `net/http` server and an `HTTPHandle<Name>` per server, standard library
  only) and a `package main` per server. Static files are written into the
  main package as strings (`go:embed` can't reach outside its directory), so
  rebuild after changing one.
- **Rust**: `http.rs` in the module (the JSON codec, a std-only server and a
  `handle_<name>` per server) and the server's executable under the crate's
  `src/`.
- **Python** uses `ThreadingHTTPServer` and embeds static text in its
  generated launcher. Rebuild after changing a static file.

The demo's `[http.api]` serves `examples/demo/sys/api.sys`:

```
curl -s localhost:8099/api/measure -d '{"shapes": ["Dot", {"Circle": 1}, {"Rect": {"w": 2, "h": 3}}]}'
curl -s 'localhost:8099/api/hello?name=vandorf&times=2'
```
