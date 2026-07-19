# nix-cargo

Nix builder for Rust projects with per-crate derivations. Parses `Cargo.lock`
and a registry index snapshot at evaluation time: no import-from-derivation,
no generated `Cargo.nix`, no cargo inside the sandbox (rustc is invoked
directly by a nushell driver). Compiled dependency crates are shared between
all projects in the repo and cacheable per crate+version+features.

Design, milestones, benchmarks, and the nocargo landmine audit:
[PLAN.md](./PLAN.md). Headline numbers (rust/systemd, 100 members, 322
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
   `DEP_*_*`, `cargo:rustc-*` directives, links metadata, proc-macros).
3. The registry metadata that is not in the lock (dep kinds, features,
   optionality, cfg gates) comes from a committed mini-index snapshot
   produced by `tools/snapshot-index.nu` from the sparse index.
4. `[profile.release]`/`[profile.dev]` from the workspace root are honored:
   `lto`, `strip`, `panic`, `codegen-units`, `debug`.

## Usage

In a flakelight package definition:

```nix
packages.my-tool = {lib, ...}:
  lib.buildCargoProject {
    src = ./.;
    index = ../../nix/lib/cargo/index;
  };
```

After updating a `Cargo.lock`, refresh the snapshot:

```console
nu nix/lib/cargo/tools/snapshot-index.nu nix/lib/cargo/index <path>/Cargo.lock
```

Verify resolution against cargo (any project, or a sweep):

```console
nix shell nixpkgs#cargo -c nu nix/lib/cargo/tools/diff-cargo.nu rust/xz
nix shell nixpkgs#cargo -c nu nix/lib/cargo/tools/diff-cargo.nu sweep rust/*/
```

### Parameters

| Parameter | Default | Description |
|---|---|---|
| `src` | required | Source root containing the workspace |
| `index` | required | Registry index checkout (snapshot or full crates.io index) |
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
| `crateOverrides` | `{}` | Per-crate derivation attr merges, e.g. `{liblzma-sys = {nativeBuildInputs = [pkg-config]; buildInputs = [xz];};}` |
| `rootAttrs` | `{}` | Extra derivation attrs for the root output (`postInstall`, `setupHook`, ...) |
| `meta` | `{}` | Nixpkgs meta for the root output |

`lib.cargoLib` exposes the pure resolution primitives (`semver`, `cfg`,
`lock`, `index`, `manifest`, `resolve`) for tests and advanced use.

A fast-iteration example combining the knobs (see `rust/systemd`):
`rust-systemd-dev` builds with `release = false`, a nightly toolchain with
the cranelift codegen backend, and wild linking; the whole 100-member
workspace cold-builds in under a minute and single-member edits rebuild in
seconds.

## Not supported (yet)

Cross-compilation, `[patch]`, running test/bench targets (covered by
per-project sandbox checks until a store-backed registry lands; see
PLAN.md), rmeta pipelining for cold-build speed. `rust/perl` is broken for
reasons predating this library (its build.rs references an absolute
dev-machine path).

## Files

- `default.nix` flakelight module: `buildCargoProject` (wild-linked by
  default) and `cargoLib` via `perSystemLib`
- `lib/` pure-eval resolution (builtins only)
- `build/` per-crate rustc driver (`crate-builder.nu`) and derivation
  wrappers
- `tools/snapshot-index.nu` sparse-index snapshotter
- `tools/diff-cargo.nu` differential oracle against `cargo tree`
- `index/` committed snapshot covering this repo's lockfiles
- `tests/` eval unit tests (`nix eval -f nix/lib/cargo/tests/<mod>.nix`)
- `checks.nix` flake checks: `cargo-lib`, `cargo-build-wclip`,
  `cargo-build-xz` (build individually; never `nix flake check` here)
