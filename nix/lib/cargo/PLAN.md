# nix-cargo: pure-eval Rust builds

Build Rust projects from `Cargo.lock` with per-crate Nix derivations, using
neither import-from-derivation nor code generation. A reusable library in the
style of `nix/lib/deno` (parse the lock at eval time, fetch with FODs, build in
the sandbox), exposed through the flakelight `perSystemLib` module.

Status: in progress. See milestones at the bottom.

## Why

- `rustPlatform.buildRustPackage` + `cargoLock` (current setup in `rust/*`)
  builds the whole dependency tree inside one derivation: no sharing between
  projects, any change rebuilds everything.
- crane/naersk split deps/workspace into two derivations: better, but a single
  dependency bump still invalidates the whole deps artifact.
- crate2nix gets per-crate derivations but needs generated `Cargo.nix` files
  (or IFD via its `tools.nix`).
- nocargo (oxalica) proved the pure-eval approach works but is alpha and
  dormant since ~2024.

Per-crate derivations mean: compiled crates shared across all Rust projects in
this repo (37+ of them, heavily overlapping dependency sets), cacheable in
cachix per crate+version+features, and a dependency bump only rebuilds its
reverse dependencies.

## Ground rules

Allowed at eval time:

- `builtins.fromTOML` / `builtins.readFile` / `builtins.readDir` on source
  paths (workspace files, committed data, flake inputs).
- Fixed-output fetches whose hash comes from `Cargo.lock` (the lock `checksum`
  is the sha256 of the `.crate` tarball, usable directly by `fetchurl`).
- `builtins.fetchGit` with a pinned `rev` (for git dependencies; eval-time
  fetcher, not IFD).

Banned:

- Reading any derivation output at eval time (IFD).
- Running cargo at eval time, or committing generated Nix.
- Unpinned network access at eval time.

## Information architecture

| Needed | Lives in | Obtained via |
|---|---|---|
| Resolved versions, checksums, package graph | `Cargo.lock` | `fromTOML` at eval |
| Crate sources | crates.io CDN | `fetchurl` FOD, hash from lock |
| Dep edge metadata (kind, optional, features, `default-features`, `cfg()` targets, renames), per-crate feature tables | registry index (not in the lock) | pinned index data at eval (see below) |
| Workspace manifests | local `Cargo.toml` | `fromTOML` at eval |
| Git dep manifests | git checkout | `builtins.fetchGit` + `fromTOML` |
| Build details (edition, target layout, `build.rs`, `links`) | crate tarball | parsed inside the sandbox at build time; never needed at eval |

The last row is the crux: evaluation only needs the graph shape and feature
assignment. Everything else moves to build time where reading files is free.

## Index sourcing

The registry index is the one input that is neither in the repo nor derivable
from the lock. Two supported strategies:

1. **Snapshot (default here).** A committed mini-index containing only the
   exact `name@version` entries appearing in the repo's lockfiles, in the
   standard index directory layout (`1/`, `2/`, `3/c/`, `ab/cd/name`), one
   JSON line per locked version. Produced by `tools/snapshot-index.nu` from
   the sparse index (`index.crates.io`). Small (one line per locked crate),
   diff-friendly, no giant flake input. Updating a lock means re-running the
   snapshot tool: morally equivalent to `cargo update` regenerating the lock,
   and it is data, not generated code.
2. **Full index as flake input.** `github:rust-lang/crates.io-index` with
   `flake = false`. Fully pure and covers any lockfile with zero per-project
   steps, at the cost of a very large input. Supported by taking the index
   path as an argument; not wired into this repo's flake by default.

The library takes `index` as a plain path argument so both work, and tests use
tiny hand-trimmed fixtures.

Index entry schema notes: JSON lines with `name`, `vers`, `deps[]` (`name`,
`req`, `features`, `optional`, `default_features`, `target`, `kind`,
`package`), `cksum`, `features`, and for schema `v >= 2` the extended syntax
(`dep:`, weak `?/`) in `features2`, which must be merged into `features`.
Handling `v2` from day one closes nocargo's oldest open bug.

## Components

```text
nix/lib/cargo/
  PLAN.md               this file
  README.md             usage docs (mirrors nix/lib/deno/README.md)
  default.nix           flakelight module: perSystemLib.{buildCargoProject,cargoLib} + checks
  lib/                  pure eval, builtins-only (no pkgs, no nixpkgs lib)
    default.nix         assembles the lib set
    semver.nix          cargo req parsing (caret, tilde, wildcard, comparators) + matching
    cfg.nix             cfg() tokenizer/parser/evaluator against a target platform
    lock.nix            Cargo.lock v3/v4 -> packages + resolved dep edges
    index.nix           index path layout, JSON-lines lookup, features2 merge
    manifest.nix        workspace manifests, member discovery, workspace inheritance
    resolve.nix         join lock+index+manifests; cfg filtering; feature fixpoint
  build/
    buildCargoProject.nix  top-level API: lock -> workspace bins/libs
    buildCrate.nix         one crate -> one derivation (rustc direct, no cargo)
    manifest-plan.py       build-time Cargo.toml -> JSON build plan (tomllib)
  tools/
    snapshot-index.nu   lockfiles -> mini-index snapshot from index.crates.io
  tests/
    *.nix               eval unit tests (nix eval -f), assert-based
    fixtures/           trimmed index files, synthetic graphs
  index/                committed snapshot for this repo's lockfiles
```

`lib/` depends only on `builtins` so unit tests run with a bare
`nix eval -f nix/lib/cargo/tests/foo.nix` and the resolver is trivially portable.

## Feature resolution

Staged for correctness with a real oracle:

- **Stage 1 (unified, resolver-v1-like).** One feature set per lock package.
  Fixpoint over a monotone state: edges activate packages, feature tables
  close over `feat`, `dep:name`, `name/feat` (activates the optional dep),
  `name?/feat` (weak: only if already activated), implicit optional-dep
  features. Over-approximates resolver v2 (unions host/target and
  cfg-disabled-elsewhere features), which is exactly what cargo's resolver v1
  shipped for years: correctness-preserving in practice, occasionally builds
  an optional dep it did not need.
- **Stage 2 (resolver v2).** Separate feature spaces for host units
  (build-deps, proc-macros) vs target units; no unification across
  non-matching `cfg()` targets; dev-deps only for workspace roots when tests
  are requested.
- **Oracle.** `cargo metadata` emits the exact resolved per-package feature
  sets. A harness diffs our eval output against it for every lockfile in the
  repo, then for top-N crates.io projects. Divergences become pinned fixture
  tests. This is the mechanism that makes the long tail tractable (agent-run
  sweeps, each failure crisp and reproducible).

Dev-dependencies of registry crates never appear in a lock and are ignored.
Dev-dependencies of workspace members are resolved only when building tests
or benches.

## Builder

One derivation per (crate, version, features, deps). No cargo inside the
sandbox, `rustc` invoked directly (buildRustCrate lineage):

- Source: `fetchurl` from `static.crates.io/crates/{name}/{name}-{version}.crate`
  with the lock checksum; workspace members use filtered local sources.
- Build-time plan: `manifest-plan.py` (python tomllib) reads the normalized
  `Cargo.toml` inside the sandbox and emits JSON (edition, lib target name and
  path, crate-types, proc-macro flag, build script presence, `links`, bins,
  required CARGO_PKG_* values). Bash + jq consume it. This is how we avoid
  needing manifests at eval time.
- `build.rs`: compiled with host deps, run with the cargo env contract
  (`OUT_DIR`, `CARGO_FEATURE_*`, `CARGO_CFG_*`, `TARGET`, `HOST`,
  `DEP_<links>_<key>` from dependencies' links metadata), stdout parsed for
  `cargo:` / `cargo::` directives: `rustc-cfg`, `rustc-env`, `rustc-link-lib`,
  `rustc-link-search`, `rustc-flags`, links metadata. Applied to the lib
  compile; links metadata persisted in `$out` for dependents.
- Lib compile: `--extern name=....rlib` for direct deps (rename-aware),
  `-L dependency=` for transitive, `--cfg 'feature="..."'`, `-C metadata=` a
  hash of (name, version, features) for symbol disambiguation,
  `--cap-lints allow`, `CARGO_*` env for `env!` users.
- proc-macro crates: `--crate-type proc-macro`, compiled for the host.
- `crateOverrides`: per-crate attrset merged into the derivation (native
  `buildInputs` for `-sys` crates, env, patches), same idea as nixpkgs
  `defaultCrateOverrides`, which we can also consume.
- Profiles: release (`-O`) and debug initially; `[profile]` fidelity later.

Deliberately deferred: cargo's rmeta pipelining. A later optimization can
split each crate into an `--emit=metadata` derivation and an rlib derivation
depending only on dep rmetas, recovering cargo's critical path at the cost of
a duplicated frontend pass. Not needed for correctness.

## Public API

```nix
# flakelight package definition
packages.my-tool = { lib, ... }:
  lib.buildCargoProject {
    pname = "my-tool";
    src = ./.;                       # contains Cargo.toml + Cargo.lock
    index = ../../nix/lib/cargo/index;   # snapshot or full index checkout
    # features = [ "foo" ];          # root features, default: default set
    # noDefaultFeatures = true;
    # bins = [ "my-tool" ];          # default: all [[bin]] targets
    # release = true;
    # crateOverrides.openssl-sys = { openssl, pkg-config, ... }: { ... };
  };
```

`lib.cargoLib` exposes the pure pieces (`semver`, `cfg`, `lock`, `index`,
`manifest`, `resolve`) for tests and advanced use.

## Testing

- Unit: assert-based eval tests per lib module, run directly via
  `nix eval -f nix/lib/cargo/tests/<mod>.nix` and wired as trivial checks.
  Individual checks build with
  `nix build .#checks.x86_64-linux.cargo-<mod>`; never `nix flake check`
  (repo rule: it OOMs).
- Corpus ladder, in order:
  1. `rust/wclip`: one dep (`libc`), exercises fetch, index lookup, default
     features, `build.rs`.
  2. `rust/xz`: 77 locked crates, edition 2024, `liblzma-sys` native linking
     via `crateOverrides`, dev-dep filtering (criterion must not be built for
     the bin).
  3. Remaining `rust/*` projects, then `ironclaw/*` (largest workspaces).
- Differential oracle: `tools/diff-cargo` compares eval-computed feature sets
  and package graphs against `cargo metadata` for each corpus lockfile.
  Later: scheduled agent sweeps over crates.io top-N, auto-filing fixture
  tests for divergences.
- End state per project: `buildCargoProject` output is compared against the
  existing `buildRustPackage` output (same binary behavior, testsuite.nix
  still passes) before switching a project over.

## Risks

- **Feature resolution divergence** builds wrong-featured crates. Mitigated
  by the oracle harness and staged rollout alongside existing packages.
- **Eval cost** on big graphs (fixpoint in the evaluator). Measure early on
  `ironclaw`; optimize representation (attrset sets, precomputed edge lists)
  before adding resolver v2 complexity.
- **build.rs long tail** (codegen'd cfgs, native probing). Contained by
  `crateOverrides` + nixpkgs `defaultCrateOverrides`; corpus ladder surfaces
  breakage per crate with a crisp failure.
- **Index snapshot drift**: a lock update without a snapshot update fails at
  eval with a clear "missing index entry" error (fail loud, easy to fix by
  re-running the tool; enforceable later as a pre-commit hook).
- **Sparse index availability**: snapshots depend on `index.crates.io` at
  tool-run time only; builds never touch it.

## Milestones

- [x] M0: PLAN.md (this file), directory scaffold.
- [x] M1: `semver.nix` + `cfg.nix` with unit tests green.
- [x] M2: `lock.nix`, `index.nix`, `manifest.nix` with unit tests green.
- [x] M3: `resolve.nix` stage 1 (unified features) green on fixtures; wclip
      and xz lockfiles resolve without error.
- [x] M4: `snapshot-index.nu` + committed `index/` covering wclip + xz.
- [x] M5: `buildCrate.nix` + `buildCargoProject.nix`: wclip builds and runs.
- [x] M6: xz `[[bin]]` builds and runs (native linking via overrides,
      dev-deps excluded); flakelight module + checks wired; README.md.
- [x] M7: differential oracle tool (`tools/diff-cargo.nu`, oracle is
      `cargo tree`, which is feature-pruned where cargo metadata is not);
      all 34 rust/* projects resolve identically to cargo (graph and
      feature sets), including virtual workspaces and cross-project path
      dependencies.
- [ ] M8: resolver v2 (host/target split); migrate first real package in-repo.

Later (not scheduled): rmeta pipelining, git dependencies, `[patch]`,
tests/benches targets, cachix population job, scheduled oracle sweeps
against crates.io top-N. (Multi-member workspaces and cross-project path
dependencies landed with M7.)

## Decision log

- 2026-07-18: Library lives at `nix/lib/cargo/`, sibling of `nix/lib/deno`;
  exposed via `perSystemLib` like the deno lib. (Initially scaffolded at
  `nix/cargo/`, moved on user correction.)
- 2026-07-18: Snapshot mini-index is the default sourcing strategy; full
  index input supported but not wired in (repo weight, flake.lock churn).
- 2026-07-18: `lib/` is builtins-only for portability and cheap tests.
- 2026-07-18: Stage feature resolution v1-unified first with `cargo metadata`
  as the oracle, resolver v2 as M8 rather than blocking first builds.
