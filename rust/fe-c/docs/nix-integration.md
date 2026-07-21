# Nix integration sketch

Written on a phone without the repo open, so **every reference to
`nix/lib/cargo` is an assumption**. §6 lists what to verify first; the shapes
below are the requirements, not the final API.

## 1. Why this project fits `nix/lib/cargo` specifically

`nix/lib/cargo` builds per-crate derivations with gradual rebuilds. Fe-C's
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

## 6. API questions to answer at the computer

1. Does `nix/lib/cargo` accept **per-crate** extra env / wrapper settings
   (`RUSTC_WRAPPER`, `RUSTFLAGS`), or only workspace-wide? The dial needs
   per-crate; if it's workspace-wide, that is the first patch.
2. Is the per-crate derivation key user-extensible? `harden` must enter the
   hash, or mode flips will silently reuse stale artifacts — a correctness
   bug, not a performance one.
3. Any existing `-Zbuild-std` / custom-sysroot support, or is
   `fe-c-sysroot-*` a new pattern to add?
4. How are pinned third-party sources vendored for offline checks — reusable
   for the corpus fixtures (which include a C dependency, `libsqlite3-sys`)?
5. Does it drive `cc`-crate builds (needed for `libsqlite3-sys`), and can
   those be pointed at an instrumented or plain toolchain per crate?
6. Is there a precedent in the repo for a derivation keyed on a *nightly
   toolchain hash* that other derivations depend on? The sysroot pattern
   probably wants to match whatever `nix/lib` already does for toolchains.

## 7. Deliberately not doing

Instrumenting C dependencies (no LLVM pass; the boundary is the boundary) ·
a second non-nix build path · per-developer sysroot builds · vendoring
`rustix` (see `../libc`).
