# Bootstrapping

vcc compiles itself: `vcc build` in the repository root turns `src/` into
the Rust in `rs/src/vcc` and `rs/src/bin`, then builds it. That generated
Rust isn't checked in, so a fresh clone needs some vcc to start from.

## From the reference compiler

Up to commit `caf0938`, the repository also held the **reference
compiler**: a hand-written Rust implementation of vandorf in `src/`, which
vcc was checked against until it reached a fixed point. Comments in `src/`
that mention "the reference" mean that code (`src/sema.rs` in the
reference, for example).

The reference reads this layout too, so it can build the first vcc:

```
git worktree add /tmp/vandorf-ref caf0938
cargo build --release --manifest-path /tmp/vandorf-ref/Cargo.toml
/tmp/vandorf-ref/target/release/vandorf build     # in this repo's root: generates rs/, builds vcc
rs/target/debug/vcc build                          # vcc rebuilds itself
git worktree remove /tmp/vandorf-ref
```

The second build regenerates the same code: the reference and vcc agree
on vcc's own sources.

## From an older vcc

Once `src/` uses a feature the reference doesn't have, the reference can't
build it any more. Then bootstrap in steps: build vcc at the last commit the
reference can build, and use that vcc to build later commits, up to the one
you want.

```
git worktree add /tmp/vandorf-old <older commit>
# ... build vcc there, from the reference or an older vcc ...
/tmp/vandorf-old/rs/target/debug/vcc build        # in this repo's root
```

A commit that adds a language feature and uses it in `src/` in the same
commit breaks this chain, so add the feature first and use it in a later
commit.

## The chain

Every commit from the reference onward, with the compiler that builds it.
Each row was checked from a fresh worktree: the compiler in the second
column builds the commit, and the vcc that comes out rebuilds the same
commit identically (the fixed point).

| Step | Compiler | From commit | Builds commit | What the commit changes |
|---|---|---|---|---|
| 0 | `cargo build --release` | (none) | `caf0938` | The last commit with the reference (`vandorf`, in the repository root; vcc was then in `selfhost/`). |
| 1 | reference (`vandorf`) | `caf0938` | `9873da7` | vcc moved to the root, the reference removed (`c9ae09f`), docs and IDE fixes. No language change. |
| 2 | vcc | `9873da7` | `974a19e` | Adds procs declared with their signature, `proc P(a: int)`, and `id(..) as N`. The old `proc P(Entry)` still parses; `src/` still uses it. |
| 3 | vcc | `974a19e` | `ce82afb` | Names a signature's generated types after its proc (`PS`, `PArgs`). `src/` still uses the old form. |
| 4 | vcc | `ce82afb` | `03f49e2` | Moves `src/` and the demo to the new form. Leaves name the new types (`RCoerceS`), so this needs a vcc with step 3's naming. |
| 5 | vcc | `03f49e2` | `ae049bc` | Drops the `proc P(Entry)` form. |
| 6 | vcc | `ae049bc` | `1a3020a` | The visualizer's signature node; the doc's signature node is type 3. |
| 7 | vcc | `1a3020a` | `ee29567` | Docs. |

Some steps can be skipped:

- The reference builds `974a19e` and `ce82afb` directly, since their
  sources are still in the old form.
- vcc from `9873da7` builds `ce82afb` directly.
- vcc from `ce82afb` builds every later commit, up to `ee29567`.

So the shortest route from the reference is two builds:

| Compiler | From commit | Builds commit |
|---|---|---|
| reference | `caf0938` | `ce82afb` |
| vcc | `ce82afb` | `ee29567` (or any commit after `ce82afb`) |

Nothing before `ce82afb` builds `03f49e2` or later. vcc from `9873da7`
can't parse the new form (`expected ) after the entry, found ':'`). vcc
from `974a19e` parses it but generates the old type names, so the
migrated leaves don't compile (``cannot find type `RCoerceS` ``).

Each step, from the repository root:

```
git worktree add /tmp/vandorf-from <compiler's commit>
# ... build the compiler there: step 0's cargo build, or the step before ...
git checkout <commit to build>
/tmp/vandorf-from/rs/target/debug/vcc build     # the reference: /tmp/vandorf-from/target/release/vandorf build
git worktree remove /tmp/vandorf-from
```

## The fixed point

After bootstrapping, or after any change to the compiler, vcc should
rebuild itself identically:

```
cp -R rs/src/vcc /tmp/vcc-gen
rs/target/debug/vcc build
diff -r /tmp/vcc-gen rs/src/vcc && echo "fixed point holds"
```
