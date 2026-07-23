# Local iteration on `fe-c-driver`

Every `nix build .#checks.x86_64-linux.fe-c-<corpus>` rebuilds the driver and the
toolchain in a sandbox — too slow for a tight edit/instrument/inspect loop. This
is how to build the driver **outside** nix and instrument a corpus fixture by
hand. (The nix checks remain the contract; this is only for iteration.)

The repo's outer devshell ships a stable `rustc` that is the *wrong* version —
`fe-c-driver` links against `rustc-dev` for the pinned nightly
(`rust-toolchain.toml`). You need that exact toolchain.

## 1. Get the pinned toolchain

From the monorepo root:

```sh
nix build --no-link --print-out-paths --impure --expr \
  'let f = builtins.getFlake (toString ./.);
       pkgs = import f.inputs.nixpkgs {
         system = "x86_64-linux";
         overlays = [ f.inputs.rust-overlay.overlays.default ];
       };
   in pkgs.rust-bin.fromRustupToolchainFile ./rust/fe-c/rust-toolchain.toml'
```

This prints a store path like `…-rust-minimal-<version>-nightly-<date>`; call it
`$TC`.

## 2. Environment (the load-bearing part)

```sh
export PATH="$TC/bin:$PATH"
SYS="$($TC/bin/rustc --print sysroot)"
export LD_LIBRARY_PATH="$SYS/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"   # runtime: librustc_driver.so
export LIBRARY_PATH="$SYS/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"            # link time: libLLVM
```

**Gotcha:** without `LIBRARY_PATH` including `$SYS/lib`, linking the driver fails
with `rust-lld: error: unable to find library -lLLVM-…`. `librustc_driver.so`
carries a native link requirement for `libLLVM-….so` (which lives in
`$SYS/lib`), and `rust-minimal` does not put that directory on the linker search
path. `LD_LIBRARY_PATH` alone is **not** enough — that is the *runtime* loader
path, not the *link-time* search path.

## 3. Build the driver — outside the repo tree

Point `CARGO_TARGET_DIR` at a scratch directory, **never** an in-repo
`target-nightly/`. Only `target/` is git-ignored, and in a jj colocated repo any
other build directory is auto-snapshotted into the working copy on the next `jj`
command, polluting your commit.

```sh
export CARGO_TARGET_DIR="$(mktemp -d)/target"
cargo build -p fe-c-driver --offline --locked
drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
```

## 4. Instrument a corpus fixture by hand

Build the fixture with `RUSTC=$drv` and scope instrumentation to the crate(s)
whose unsafe code you care about (the binary plus the vulnerable dependency):

```sh
( cd corpus/<fixture> \
    && FEC_INSTRUMENT=1 FEC_INSTRUMENT_ONLY=<bin_crate>,<dep_crate> \
       FEC_MODE=through RUSTC="$drv" CARGO_TARGET_DIR="$(mktemp -d)/t" \
       cargo build --offline --locked )
"$CARGO_TARGET_DIR/../t/debug/<bin>"    # run; look for `fe-c-violation kind=…`
```

Drop `FEC_MODE=through` for `case` mode. `FEC_DEBUG=1` makes the driver print
which bodies it instrumented and the check count — but that goes to the *rustc*
stderr, which cargo buffers and only surfaces on a non-clean compile; redirect
the build's stderr to a file to read it.

## 5. Dump the MIR the driver actually sees

The driver clones the `optimized_mir` query result, so plain nightly `rustc`
(no driver) shows you the exact shape to match:

```sh
rustc --edition 2021 -g -Zmir-opt-level=1 \
  -Zdump-mir='<fn_regex>' -Zdump-mir-dir=<dir> file.rs -o <dir>/a.out
# the final `*.runtime-optimized.after.mir` == optimized_mir's result
```

Two facts this makes concrete, both of which shape the instrumentation passes:

- **Corpus builds are debug** (opt-level 0 → `mir-opt-level` 1): the general
  `Inline` pass does **not** run (only `ForceInline`). So `s[i]` stays
  `_r = &(*slice)[i]` — a `Deref` of a `&[T]` followed by `Index`, **not** an
  inlined `&*(ptr.add(i))`. `get_unchecked` inlines only at opt-level ≥ 2. This
  is why slice / `get_unchecked` out-of-bounds catchability depends on the fixture's
  opt level (see `slice-oob` vs `partial-sort-0016`).
- **This toolchain's MIR differs from older/mainline rustc.** `NullOp` /
  `Rvalue::NullaryOp` are absent — `size_of::<T>()` is a plain `Call` to
  `core::mem::size_of`, which has no lang or diagnostic item (resolve it by
  walking `core`'s `module_children`; see `resolve_size_of` in `instrument.rs`).
  Dump real MIR to confirm a construct before synthesizing it.
