# Architecture

vcc is written in vandorf. Its control flow is `.sys` in `src/`; the few
things vandorf can't express (lexing characters, formatting text, walking
directories) are Rust funcs next to the `.sys` using them. vcc generates its
own Rust into `rs/`, and builds itself.

## The repository

```
vandorf.toml        vcc's own project: sources in src/, Rust into rs/src/vcc,
                    the vcc executable, the IDE's HTTP server ([http.ide])
                    and grove's ([http.grove])
src/                the compiler, in .sys, with its *_leaf.rs and I/O .rs
rs/                 the crate vcc generates into
  src/lib.rs          mounts the generated module
  src/vcc/            generated: types.rs, procs.rs, http.rs, mod.rs
  src/bin/            generated: vcc.rs and ide.rs
  tests/              vcc's tests, their corpus and golden files
web/                the IDE's and grove's pages, embedded in their executables
examples/demo/      a project built for Go and Rust
docs/
```

`rs/src/vcc` and `rs/src/bin` are generated, and not checked in; see
[Bootstrapping](bootstrapping.md).

## Pure procs, pure leaves, and I/O

vcc keeps to one rule: everything is pure except a few interfaces.

- **Procs** (`.sys`) are the control flow: parsing, checking, laying out and
  generating are graphs of steps, with loops as back edges and data threaded
  through the steps' outputs.
- **Leaves** are funcs implemented in a `*_leaf.rs` next to the `.sys` that
  declares them (`leaf.rs`, `sema/sema_leaf.rs`, `gen/gen_leaf.rs`, ...).
  Each is pure: its result depends only on its arguments. A leaf fails with
  a message, which fails the proc that called it.
- **I/O** goes through interfaces (`io.sys`, implemented in `io.rs`; the
  IDE's in `ide/ide.sys` and `ide/ide_io.rs`): reading and writing files,
  running commands, the console, terminals and the IDE's caches.

## The pipeline

```
source ─▶ lex ─▶ parse ─▶ sema ─┬─▶ doc ─▶ the visualizer document (JSON)
                                └─▶ gen ─▶ Go and Rust
```

| Stage | Files | What it does |
|---|---|---|
| Lexing | `lex.sys`, `leaf.rs` | Tokens, with spans, each carrying the comment lines directly above it (a declaration's doc). A leaf, since vandorf strings have no character access. |
| Parsing | `ast.sys`, `parse*.sys` | Recursive descent over the tokens, into the AST (`ast.sys`): what was written, with spans. |
| Sema | `sema/` | Typedefs and their instances (`typedefs.sys`), declarations and name resolution (`decls.sys`), expressions and their types (`exprs.sys`), switches and exhaustiveness (`switch.sys`), and each proc's graph with its for each regions and the check that reads see nodes that ran (`graph.sys`). The result is a `Program` (`program.sys`). |
| Layout | `doc/` | Each proc as a grid of rows, for the diagram. |
| Codegen | `gen/` | The type table shared by both backends (`table.sys`): one name per struct or enum shape. Then Rust (`rust.sys`, `rust_procs.sys`, `rust_http.sys`) and Go (`go.sys`): types, a state machine per proc, servers and mains. |
| Commands | `main.sys`, `cli/` | The command line, projects and manifests (`project.sys`), and `check`, `doc`, `build` and `test` (`build.sys`), with the check of each implementation's signature and doc against its declaration. |
| IDE | `ide/` | The IDE's HTTP routes, each a raw route to its own proc: pages, the file tree, compiles (with a cache of checked programs), saves and terminals. |
| grove | `grove/` | The idea tree's routes: ideas from git and their metadata, branching, discarding and layouts (`ideas.sys`), interiors and their processes (`interiors.sys`); it shares the IDE's session, gates and terminals. |

### Parsing

Every parse proc takes the tokens and a position, goes first to a local
`At` (the position as a `Cur`), and ends with
`id({ x = <what it parsed>, pos = <the next token> })`. Lists are loops: a
back edge to a loop head that carries the items so far. Steps that check a
token fail with "expected <what>, found <token>"; a switch's `else` arm
usually goes to one, so the error says what would have been accepted.

### Sema

Every func and proc declaration is a node, by index into `Program.nodes`,
and every proc has a graph of typed edges. A proc's entry is its signature
node, `P.P`, a local of P named like it; `id(..)` targets are nodes too,
and an `id(..) as N`'s type is found from the values passed to it before
the rest of its proc's graph is checked. Typedefs are named by index into
the program's typedefs; each instance of a generic typedef is its own. Errors
carry spans, which `vcc` renders with the line and a caret.

### Layout

Layout is a breadth-first sweep, one row per step: every node in the last
row adds its edge targets to the next row. When an edge reaches a node
already placed (and isn't a back edge), that node is pushed down, with
everything downstream of it. A back edge, one whose target already reaches
its source, is drawn upward without moving anything.

### Codegen

Both backends share the type table, so a shape has the same name in Go and
Rust. Each proc becomes a state machine with a step per node, keeping each
node's output for the steps that read it. Implementations are called through the
paths the funcs name, rebased from the `.sys` file onto the output
directory.

## Running vcc

`rs/src/bin/vcc.rs` runs the proc `Vcc` with the command line. Generated
code runs on a thread with a large stack (deep recursion through procs is
common in vcc); `.cargo/config.toml` raises the default stack for tests,
which call procs on libtest's own threads.

The IDE is `[http.ide]` in `vandorf.toml`: `rs/src/bin/ide.rs` is its
generated server, and every route is a raw route to a proc in `src/ide/`.
