# nix-lib-cargo

Nix builder for Rust projects with per-crate derivations. Parses `Cargo.lock`
and a registry index at evaluation time: no generated `Cargo.nix`, no cargo
inside the sandbox (rustc is invoked directly by a nushell driver). Pass a
committed index snapshot to keep evaluation free of import-from-derivation, or
omit `index` and the mini-index is reconstructed from the crates' own
tarballs by a pure IFD derivation (no network, no cargo). Compiled dependency
crates are shared between all projects in the repo and cacheable per
crate+version+features.

Design, milestones, benchmarks, and the nocargo landmine audit:
[PLAN.md](./PLAN.md). Headline numbers (safety/oxidized/systemd, 100 members, 322
locked crates): a one-line edit rebuilds in seconds instead of the full
workspace recompile `buildRustPackage` pays; cranelift dev builds go
~2.4x faster cold; release links use the wild linker by default.

## How it works

1. `lib/` (pure builtins) parses `Cargo.lock`, workspace manifests, and the
   registry index, then runs cargo-style feature resolution (optional deps,
   `dep:`, weak `?/` features, cfg-gated targets, renames) as a monotone
   fixpoint. Resolution is oracle-verified identical to `cargo tree` for
   every Rust project in this repo.
2. Each resolved crate becomes one derivation: registry sources come from
   `fetchurl` keyed by the lock checksum, git dependencies from
   `builtins.fetchGit` keyed by the locked revision (packages are located
   inside the checkout, including workspace members). The manifest is
   parsed inside the sandbox and `build/crate-builder.nu` drives rustc and
   the cargo build-script protocol (`OUT_DIR`, `CARGO_FEATURE_*`,
   `DEP_*_*`, `cargo:rustc-*` directives, links metadata, proc-macros). The
   `rustc --print cfg` set is computed once per toolchain and shared across
   every build-script sandbox rather than recomputed per crate.
3. The registry metadata that is not in the lock (dep kinds, features,
   optionality, cfg gates) comes from an index checkout. By default
   `tools/tarball-index.nu`
   rebuilds the mini-index inside a derivation by reading each crate's
   published `Cargo.toml` out of the same fixed-output `.crate` tarballs the
   build already fetches, and `lib/index.nix` reads that output at eval time.
   That is the library's one import-from-derivation, and it stays pure: the
   tarballs are content-verified by the lock checksums, so no network and no
   sandbox relaxation are involved. Passing `index` skips it: a committed
   mini-index snapshot (produced by `tools/snapshot-index.nu` from the sparse
   index: small, diff-friendly, no IFD), or a full crates.io index checkout.
4. `[profile.release]`/`[profile.dev]` from the workspace root are honored
   (`lto`, `strip`, `panic`, `codegen-units`, `debug`, including
   `debug = "line-tables-only"`), plus per-package overrides:
   `[profile.<p>.package."<name>"]` targets one crate and
   `[profile.<p>.package."*"]` applies to every dependency (workspace members
   keep the base profile unless named explicitly).
5. Cross-compilation is minimal but real: pass `crossTarget` (a platform key
   like `"aarch64-linux"`) and a `crossCC`. Resolution runs dual-platform
   (normal edges filtered by the target cfg, build/dev edges by the host cfg);
   libraries and binaries compile with `--target` and link through the cross
   cc, while build scripts and proc-macros compile in a parallel host closure.
   Needs a toolchain carrying the target's std.

## Usage

In a workspace package definition:

```nix
packages.my-tool = {lib, ...}:
  lib.buildCargoProject {
    src = ./.;
  };
```

This is what the tree does: the builder reconstructs the mini-index from the
crate tarballs by IFD, so a `Cargo.lock` change needs no second commit and a
project holds no path out of its own directory. The cost is one
import-from-derivation on the eval path, which means evaluating a system this
machine cannot build for now needs a builder for it.

Pass `index` to avoid that - a full crates.io index checkout, or a snapshot
built from the locks that need it - and eval stays pure:

```nix
index = ./cargo-index;
```

```console
nu platform/nix/lib/lib/cargo/tools/snapshot-index.nu <out-dir> <path>/Cargo.lock
```

Nothing in this tree does: the snapshot it used to share was 1361 files that
had to be recommitted whenever a lock moved.

Verify resolution against cargo (any project, or a sweep):

```console
nix shell nixpkgs#cargo -c nu platform/nix/lib/lib/cargo/tools/diff-cargo.nu safety/oxidized/xz
nix shell nixpkgs#cargo -c nu platform/nix/lib/lib/cargo/tools/diff-cargo.nu sweep rust/*/
```

### Parameters

| Parameter | Default | Description |
|---|---|---|
| `src` | required | Source root containing the workspace |
| `index` | `null` | Registry index checkout (snapshot or full crates.io index); `null` reconstructs the mini-index from the crate tarballs by IFD |
| `manifestDir` | `""` | Workspace manifest location inside `src` (for path deps on sibling projects) |
| `lockFile` | `src/manifestDir/Cargo.lock` | Lock file override |
| `pname` | root crate name | Package name (required for multi-root workspaces) |
| `version` | crate version | Version for multi-root aggregate outputs |
| `features` | `[]` | Root package features |
| `noDefaultFeatures` | `false` | Disable the root default feature |
| `roots` | all members | Workspace members to build (several members produce a symlinkJoin of their outputs) |
| `bins` | all | Subset of `[[bin]]` names to build |
| `release` | `true` | Release vs dev profile |
| `linker` | `pkgs.wild` | Linker exposed to cc as `ld`; `null` for the stdenv default |
| `toolchain` | import-time `rustc` | Toolchain override (e.g. `rust-bin.nightly...` with the cranelift component) |
| `rustcFlags` | `[]` | Extra flags for every rustc invocation (e.g. `["-Zcodegen-backend=cranelift"]`) |
| `crossTarget` | `null` | Cross-compile to a platform key (e.g. `"aarch64-linux"`); build scripts and proc-macros stay host, needs a toolchain with the target std |
| `crossCC` | `null` | Cross C compiler for linking (e.g. `pkgsCross.aarch64-multiplatform.stdenv.cc`) |
| `pipeline` | `false` | Experimental rmeta pipelining; blocked on upstream rustc (see PLAN) |
| `runTests` | `false` | Compile and run each root member's test targets (unit + `tests/*.rs`) with dev-deps; exposed as `passthru.tests.<member>` |
| `crateOverrides` | `{}` | Per-crate derivation attr merges, e.g. `{liblzma-sys = {nativeBuildInputs = [pkg-config]; buildInputs = [xz];};}` |
| `rootAttrs` | `{}` | Extra derivation attrs for the root output (`postInstall`, `setupHook`, ...) |
| `meta` | `{}` | Nixpkgs meta for the root output |

`lib.cargoLib` exposes the pure resolution primitives (`semver`, `cfg`,
`lock`, `index`, `manifest`, `resolve`) for tests and advanced use.

A fast-iteration example combining the knobs (see `safety/oxidized/systemd`):
`oxidized-systemd-dev` builds with `release = false`, a nightly toolchain with
the cranelift codegen backend, and wild linking; the whole 100-member
workspace cold-builds in under a minute and single-member edits rebuild in
seconds.

## Patches

`[patch]` source overrides in the workspace root are honored. A patch
pointing a crate at a git source resolves through the locked git revision
like any git dependency; a patch pointing at a local path (within `src`) is
discovered and built from that directory. Both are picked up automatically
from the manifest, no parameter needed.

## Tests

`runTests = true` compiles and runs each root member's test targets in the
sandbox: unit tests (the lib and every bin, compiled with `--test`) and
integration tests (`tests/*.rs`, linked against the crate's lib). Resolution
for the test graph includes dev-dependencies and stays separate from the
package's own build, so enabling it never changes the package derivations. A
nonzero test exit fails the build. Results are per member under
`passthru.tests.<crate-name>` (e.g. `nix build .#my-tool.tests.my-tool`).

## Not supported (yet)

Running bench targets, rmeta pipelining for cold-build speed. `safety/oxidized/perl` is
broken for reasons predating this library (its build.rs references an
absolute dev-machine path).

## Files

- `default.nix` workspace module: `buildCargoProject` (wild-linked by
  default) and `cargoLib` via `perSystemLib`
- `lib/` pure-eval resolution (builtins only)
- `build/` per-crate rustc driver (`crate-builder.nu`) and derivation
  wrappers
- `tools/snapshot-index.nu` sparse-index snapshotter
- `tools/tarball-index.nu` IFD fallback: rebuilds the mini-index from crate
  tarballs when `index` is omitted
- `tools/diff-cargo.nu` differential oracle against `cargo tree`
- `index/` committed snapshot covering this repo's lockfiles
- `tests/` eval unit tests (`nix eval -f platform/nix/lib/lib/cargo/tests/<mod>.nix`)
- `checks.nix` flake check `cargo-lib` (build it directly; never `nix flake
  check` here)
