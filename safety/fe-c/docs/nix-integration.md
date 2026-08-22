# Nix integration sketch

Written on a phone without the repo open, so **every reference to
`platform/nix/lib/lib/cargo` is an assumption**. §6 lists what to verify first; the shapes
below are the requirements, not the final API.

## 1. Why this project fits `platform/nix/lib/lib/cargo` specifically

`platform/nix/lib/lib/cargo` builds per-crate derivations with gradual rebuilds. Fe-C's
hardening dial is *also* per-crate. They compose exactly:

```text
harden = "through";   # crate attribute, alongside features/profile
```

Flipping one crate's mode invalidates that crate and its dependents only —
not the world. That is the difference between a usable dial and a
recompile-everything flag, and it is the reason not to reach for a
whole-workspace `RUSTFLAGS` approach.

## 2. Outputs

```text
packages:
  cargo-fe-c            # cargo subcommand + RUSTC_WRAPPER
  cementite             # runtime crate (also a lib output for linking)
  fe-c-driver           # rustc-as-a-library driver

  fe-c-sysroot-<mode>-<target>
      # instrumented core/alloc/std via -Zbuild-std.
      # Keyed on: nightly hash x mode x target x cementite hash.
      # THE expensive artifact. Built once per nightly bump, served from
      # harmonia. Everything else is cheap by comparison.

  libc-sysroot-<mode>   # ../libc substrate, same keying (phase P2)
```

The sysroot derivations are the whole reason nix is worth the wiring here: a
`-Zbuild-std` rebuild per developer per branch is intolerable; a cache hit is
free.

## 3. Checks (`nix flake check` = CI, run by spindle)

Cheap tier, runs on every PR:

```text
fmt, statix, deadnix, clippy
unit                    # cementite + driver unit tests
ui                      # trybuild-style driver diagnostics
miri-runtime            # cementite's own unsafe under Miri
```

Corpus tier — the interesting one. Each entry is a pinned vulnerable crate
plus a reproducer, asserted to abort with a specific report:

```text
corpus-lru-0130         # temporal, heap        (docs/traces/rustsec-2021-0130.md)
corpus-rusqlite-0128    # temporal, stack, FFI  — also the mixed-language smoke test
corpus-smallvec-0003    # spatial, provenance   — canary for the I10 regression
```

Each needs its `-control` twin (patched version, must run clean) and, per the
smallvec trace, a `-unmapped` variant so a test can't accidentally pass via
segfault. All sources vendored so checks stay pure and offline.

Expensive tier, nightly rather than per-PR:

```text
false-positive          # serde/regex/hashbrown own test suites under case
selfhost                # cementite + driver built under case
differential            # per docs/both-modes.md: violations through-catches
                        # that case misses must be a documented elision gap
bench                   # criterion; non-gating; baselines are ASan and Fil-C,
                        # never our own fast mode (I5)
```

## 4. devShell

Pinned nightly (`rust-toolchain.toml`, shared with `../libc`) plus
`rustc-dev`, `rust-src`, `miri`, `cargo-fe-c` from the flake, and just recipes
that shell out to nix so there is one execution path, not two.

## 5. Pipelines (`.tangled/`)

| Pipeline | Trigger | Does |
| -------- | ------- | ---- |
| `check` | PR | cheap + corpus tiers |
| `nightly-bump` | weekly | bump `rust-toolchain.toml`, rebuild sysroots, run full checks, open PR. Breakage budget: one sitting; if exceeded, reduce the coupling surface (PLAN §8) |
| `bench-report` | nightly | expensive tier + report artifact |
| `upstream-diff` | quarterly | `../libc` vendored trees vs upstream HEAD (libc PLAN §3) |

## 6. API questions — answered at the computer (Task A1, 2026-07-21)

Verified against `platform/nix/lib/lib/cargo` as of the `fe-c/v0` branch point.

1. **Per-crate env / wrapper settings: no, workspace-wide only.**
   `buildCargoProject` accepts `rustcFlags` and `toolchain` for the whole
   graph (`build/buildCargoProject.nix`); `crateOverrides.<name>` merges
   derivation attrs (`nativeBuildInputs`, `buildInputs`) but there is no
   per-crate rustc-flags or wrapper knob, and `build/crate-builder.nu`
   invokes `rustc` directly (no `RUSTC_WRAPPER` concept at all). As
   predicted, per-crate is the first patch; it is not needed until the
   per-crate dial lands (phase C/v1), since v0 instruments whole
   workspaces. Patch seam: thread a per-crate flags/driver attribute from
   `crateOverrides` through `build/buildCrate.nix` into the builder script.
2. **Per-crate derivation key: not user-extensible today.** The artifact
   key is `hashOf = sha256("${id}:${features}:${effectiveRustcVersion}:v1")`
   (`build/buildCargoProject.nix`, ~line 220). `harden` must be added to
   that string in the same patch that adds the per-crate knob, or mode
   flips reuse stale staged artifacts. Until then the whole-workspace
   `toolchain`/`rustcFlags` do enter each derivation's inputs, so
   whole-workspace mode flips rebuild correctly.
3. **`-Zbuild-std` / custom sysroots: none.** Only `--extern proc_macro`
   sysroot linking exists. `fe-c-sysroot-<mode>-<target>` is a new pattern
   (Task D1).
4. **Vendoring: solved and reusable for the corpus.** `Cargo.lock` is
   parsed in pure nix (`lib/lock.nix`), each crate fetched with `fetchurl`
   from `static.crates.io` by lockfile checksum, and registry metadata
   is rebuilt from those same tarballs by `tools/tarball-index.nu`, so a
   lockfile is the whole input. Corpus fixtures pin vulnerable versions in
   their own lockfiles; checks stay offline, at one IFD on the eval path.
5. **`cc`-crate builds: yes.** `build/crate-builder.nu` runs `build.rs`
   with the full `cargo:` directive protocol; native deps arrive via
   `crateOverrides` (see `safety/oxidized/xz`'s `liblzma-sys` override). Pointing a
   *single* crate at an instrumented vs plain toolchain is the same
   missing per-crate knob as Q1.
6. **Toolchain-keyed derivations: precedent exists.** `toolchain` accepts
   any drv providing `bin/rustc`; `safety/oxidized/systemd`'s `oxidized-systemd-dev`
   already passes `rust-bin.nightly.latest` from the `rust-overlay` flake
   input, and `effectiveRustcVersion` enters every artifact key. Fe-C pins
   `nightly-2026-06-29` (the newest date the locked `rust-overlay` input
   carries; miri/rustc-dev/rust-src all available there) via
   `rust-bin.fromRustupToolchainFile ./rust-toolchain.toml`, so
   `rust-toolchain.toml` stays the single source of truth. Sysroot
   derivations (D1) should key on the toolchain drv + mode + target +
   cementite hash, consistent with this.

## 7. Deliberately not doing

Instrumenting C dependencies (no LLVM pass; the boundary is the boundary) ·
a second non-nix build path · per-developer sysroot builds · vendoring
`rustix` (see `../libc`).
