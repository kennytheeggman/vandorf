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

## The fixed point

After bootstrapping, or after any change to the compiler, vcc should
rebuild itself identically:

```
cp -R rs/src/vcc /tmp/vcc-gen
rs/target/debug/vcc build
diff -r /tmp/vcc-gen rs/src/vcc && echo "fixed point holds"
```
