# nix-cargo

Nix builder for Rust projects with per-crate derivations. Parses `Cargo.lock`
and a registry index snapshot at evaluation time: no import-from-derivation,
no generated `Cargo.nix`, no cargo inside the sandbox (rustc is invoked
directly). Compiled dependency crates are shared between all projects in the
repo and cacheable per crate+version+features.

Design and roadmap: [PLAN.md](./PLAN.md).

## How it works

1. `lib/` (pure builtins) parses `Cargo.lock`, workspace manifests, and the
   registry index, then runs cargo-style feature resolution (optional deps,
   `dep:`, weak `?/` features, cfg-gated targets) as a monotone fixpoint.
2. Each resolved crate becomes one derivation: the source comes from
   `fetchurl` keyed by the lock checksum, the manifest is parsed inside the
   sandbox (`build/manifest_plan.py`), and `build/crate_builder.py` drives
   rustc and the cargo build-script protocol (`OUT_DIR`, `CARGO_FEATURE_*`,
   `DEP_*_*`, `cargo:rustc-*` directives, links metadata).
3. The registry metadata that is not in the lock (dep kinds, features,
   optionality, cfg gates) comes from a committed mini-index snapshot
   produced by `tools/snapshot-index.nu` from the sparse index.

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

### Parameters

| Parameter | Default | Description |
|---|---|---|
| `src` | required | Source directory containing `Cargo.toml` + `Cargo.lock` |
| `index` | required | Registry index checkout (snapshot or full crates.io index) |
| `lockFile` | `src + "/Cargo.lock"` | Lock file override |
| `pname` | root crate name | Package name |
| `features` | `[]` | Root package features |
| `noDefaultFeatures` | `false` | Disable the root default feature |
| `roots` | sole member | Workspace member to build |
| `bins` | all | Subset of `[[bin]]` names to build |
| `release` | `true` | opt-level 3 vs 0 + debuginfo |
| `crateOverrides` | `{}` | Per-crate derivation attr merges (e.g. `buildInputs` for `-sys` crates) |
| `meta` | `{}` | Nixpkgs meta for the root |

`lib.cargoLib` exposes the pure resolution primitives (`semver`, `cfg`,
`lock`, `index`, `manifest`, `resolve`) for tests and advanced use.

## Files

- `default.nix` flakelight module exposing `buildCargoProject` and `cargoLib`
- `lib/` pure-eval resolution (builtins only)
- `build/` per-crate rustc builder
- `tools/snapshot-index.nu` sparse-index snapshotter
- `index/` committed snapshot for this repo's lockfiles
- `tests/` eval unit tests (`nix eval -f nix/lib/cargo/tests/<mod>.nix`)
