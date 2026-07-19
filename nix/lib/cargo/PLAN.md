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

- Reading any derivation output at eval time (IFD). One opt-in exception:
  when `index` is omitted, the mini-index is reconstructed from the crate
  tarballs by a pure derivation and read via IFD (see Index sourcing). Passing
  a committed snapshot keeps evaluation strictly IFD-free.
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

The registry index metadata is not in the lock and not in the repo. Three
supported strategies:

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
3. **IFD from the crate tarballs (`index` omitted).** The metadata *is*
   derivable from the lock after all: each crate's published `Cargo.toml` is
   what crates.io turns into its index entry, and we already fetch every
   `.crate` tarball as a fixed-output derivation (hash from the lock).
   `tools/tarball-index.nu` reads those `Cargo.toml`s inside a derivation and
   re-emits the mini-index; `lib/index.nix` reads the output at eval time.
   Pure and cacheable (content-verified tarballs, no network, no sandbox
   relaxation) but pays one IFD on the eval path. Removes the per-project
   snapshot chore for callers that accept IFD.

   Verified byte-for-byte: on the 322-crate `rust/systemd` workspace an
   IFD-built index yields derivations identical to the committed snapshot for
   all 322 crates (the generator sorts index deps by name to match cargo's
   ordering; edge order otherwise leaks into crate build inputs). Cost, tarballs
   already fetched (a build fetches them anyway): the ~1 MB / 224-file mini-index
   derivation builds in ~5-6s, and once built the eval overhead is nil (warm eval
   ~4.7s vs ~5.8s for the committed snapshot, within noise). This supersedes the
   older impure-harness figure below, which measured whole-flake eval overhead
   rather than index generation.

The library takes `index` as a plain path argument (or null for strategy 3),
and tests use tiny hand-trimmed fixtures.

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

Done since: `[patch]` source overrides (path patches discovered through the
external-path walk; git patches resolve via the locked git source), and
`runTests` — per-member unit + `tests/*.rs` integration test compilation and
execution in the sandbox, on a separate dev-inclusive resolution so the
package's own derivations are unchanged.

Later (not scheduled): rmeta pipelining, bench targets and doctests (a
store-backed fake sparse registry, pnpm-style, is the right tool and doubles
as the dyn-drvs planner later), cachix population job, scheduled oracle
sweeps against crates.io top-N.
(Multi-member workspaces and cross-project path deps landed with M7;
git deps, profile fidelity, and exclude globs landed on the landmines
branch.)

## Benchmark (rust/systemd, 2026-07-18)

Largest workspace in the repo: 100 members, 322 locked crates, 80 binaries,
measured on 22 cores (max-jobs 22) against `buildRustPackage` + `cargoLock`.
Incremental = one comment line appended to `crates/cat/src/main.rs`.

| Scenario | buildRustPackage | buildCargoProject |
|---|---|---|
| Cold build, full workspace | 2m32s | ~5m08s |
| One-line edit in one member | 2m11s | 3.8s |
| No-op rebuild | ~1s | 1.3s |

Cold is ~2x slower: per-derivation sandbox setup for ~260 crates plus the
lost cargo rmeta pipelining (the planned split-rmeta optimization attacks
both). Incremental is ~34x faster: only the edited member and the symlink
join rebuild, while buildRustPackage recompiles all 322 crates for any
source change. Cold builds happen once per dependency-set change and are
amortized further by the shared per-crate binary cache across the repo's
34 Rust projects; the incremental case is every development iteration.
(The new cold number is derived: 7m40s measured through an `--impure`
harness minus its measured 2m32s fixed eval overhead.)

### Wild linker and cranelift (added later on 2026-07-18)

Same systemd workspace, harness overhead subtracted (the fixed ~152s
impure-eval cost of the benchmark harness; flake-attr builds pay seconds).

Release builds, wild linker (`linker = pkgs.wild`, adopted for systemd):

| Scenario | stock | wild |
|---|---|---|
| Cold full build | ~5m08s | ~3m41s |
| One-line incremental (marginal) | ~2.5s | ~1.7s |

Dev builds (`release = false`), nightly + cranelift
(`toolchain = rust-bin.nightly...` with the codegen-cranelift component,
`rustcFlags = ["-Zcodegen-backend=cranelift"]`):

| Scenario (net) | stock | cranelift | cranelift + wild |
|---|---|---|---|
| Cold full build | 2m03s | 52s | 49s |
| One-line incremental (marginal) | <1s | <1s | <1s |

Cranelift cuts dev codegen ~2.4x across the 260-crate graph; wild's win
concentrates in release links (80 bins + proc-macro dylibs) and adds
little on top for debug links. Incremental rebuilds are sub-second in all
variants: the per-crate model makes the edited member the only work.
Nightly-with-cranelift stays reproducible because rust-overlay pins its
manifest set in flake.lock.

## nocargo landmine audit (2026-07-19)

Every issue in oxalica/nocargo's tracker checked against this library:

| nocargo | Landmine | Status here |
|---|---|---|
| #16 | phantom build-script detection on plain bins | defused (wclip, corpus) |
| #7 | build-deps missing from build.rs compile | defused (buildExterns; zstd-sys, liblzma-sys) |
| #9 | index path case-folding (CoreFoundation-sys) | defused (relPath lowercases, unit test) |
| #4 | index schema v2 / features2 | defused day one (merge + v>2 guard) |
| #19, #21 | time-macros / deranged proc-macros | defused, live-tested: `time` project resolves identically to cargo, builds, macro output correct. Root cause difference: manifest-driven crate-type detection instead of nocargo's hardcoded proc-macro list |
| #10 | manifest shape polymorphism ("list while a set was expected", trigger never diagnosed) | hardened on the landmines branch: defensive coercions in both parsers + per-crate error context; irreducible remainder guarded by oracle and corpus |
| #5 | LTO unsupported | fixed on the landmines branch: profile lto/strip/panic/codegen-units/debug honored; xz shrank 38%, wclip 40% |
| #3, #14 | git deps (subfolders, workspaces) | fixed on the landmines branch: fetchGit by lock rev, package located via workspace or tree scan; live-tested against a serde workspace member |
| README | cross-compilation | not attempted |
| README | workspace `exclude` semantics | fixed on the landmines branch: glob matching (`*`, `**`, `?`) |

## Landmine branch plan (nix-lib-cargo-landmines, 2026-07-19)

Closing every actionable row of the audit above, in dependency order:

1. **Shape hardening (#10 class).** Cargo.toml is polymorphic in ways the
   undiagnosed nocargo crash likely hit. Coerce defensively in both
   parsers (eval `manifest.nix`, sandbox `crate-builder.nu`):
   `crate-type` string -> list, single `[bin]` table -> list, `authors`
   string -> list, `edition`/`rust-version` numbers -> strings. Wrap
   normalization in `addErrorContext` naming the crate so any surviving
   shape surprise identifies itself. Fixture tests per shape.
2. **Profile fidelity (#5).** Honor `[profile.release]`/`[profile.dev]`
   from the workspace root: `lto` (deps get `-C embed-bitcode=yes`, bins
   get `-C lto=thin|fat`), `strip`, `panic` (applied everywhere except
   proc-macros and build scripts, matching cargo), `codegen-units`,
   `debug` levels. Only explicit keys change behavior; defaults stay as
   today. Verified by xz/wclip (both set `lto = true`, `strip = true`):
   binaries must shrink and still pass their roundtrips.
3. **Exclude globs.** `workspace.exclude` entries match as globs
   (`*` within a segment, `**` across), not literals.
4. **Git dependencies (#3, #14).** `builtins.fetchGit` by the lock's
   resolved rev (pure; memoized per url+rev), package located in the
   checkout by trying: root package, workspace member (nocargo #14),
   then a bounded tree scan (nocargo #3); workspace inheritance resolved
   against the checkout's own root. Builder sources come from the
   checkout. Registry deps of git crates are already covered by the
   snapshot (they appear in the lock). Live-tested against a git dep on
   a workspace member of the serde repository, oracle-diffed.

Explicitly out of scope for this branch: cross-compilation (a milestone
needing per-unit host/target features and target toolchains, not a fix)
and the irreducible remainder of #10 (unknown unknowns; the oracle and
corpus remain the guard).

## Performance roadmap (2026-07-19)

Every known remaining lever, with expected magnitude and status. Each item
lands only with a benchmark proving it helped.

| # | Lever | Expected | Status |
|---|---|---|---|
| P1 | Dev: `-Zthreads` parallel rustc frontend (nightly) | 20-40% off frontend-bound crates | benchmarked, not adopted: -3% cold, noise elsewhere; crate-level parallelism already saturates the cores (systemd dev, cranelift+wild, high-performance governor) |
| P2 | Dev: `-C prefer-dynamic` (dynamic libstd, 80 bins stop static-linking std) | large cut of dev link time | benchmarked, not adopted: -1% cold, incremental identical to no-op; wild already made links free |
| P3 | Per-package profile overrides (`[profile.dev.package."*"]` opt-level) | dev ergonomics parity with cargo | done (lib/profile.nix, unit-tested; wildcard hits dependencies, named overrides beat it, members keep base) |
| P4 | `debug = "line-tables-only"` support | cheaper debuginfo, keeps backtraces | done with P3 |
| P5 | Shared `rustc --print cfg` derivation (drop per-build-script subprocess) | seconds of aggregate CPU | done: wall-neutral (195.92s vs 195.88s baseline), kept for the cleaner architecture |
| P6 | Shared unpack derivation per crate version (stop re-untarring per variant) | seconds; dedups across feature variants | benchmarked and REVERTED: +14s cold (224 extra sandbox setups outweigh the shared unpack; variant-sharing too rare to pay for it) |
| P7 | rmeta pipelining: metadata drv + rlib drv per crate, dependents compile against rmeta | recovers most of the remaining ~1.45x cold gap vs cargo | BLOCKED upstream; scaffolding kept behind `pipeline = true`. Two hard findings: stable's standalone `--emit=metadata` omits the optimized MIR dependents need for codegen, and even nightly with `-Zalways-encode-mir` plus exact flag parity yields E0460 (separately-compiled rmeta and rlib get different SVHs; cargo's pipelining rmeta comes from the same invocation mid-flight, which atomic derivations cannot observe). Needs a rustc mode emitting standalone metadata bit-compatible with a later link compile; same need as Bazel-style build systems |
| P8 | Content-addressed rmeta drvs: early cutoff on impl-only dep changes | skips entire dependent subtrees; beyond cargo | blocked on P7 plus ca-derivations fleet-wide |
| P9 | Build-script run as its own derivation (keyed on build.rs + build-deps) | lib edits stop re-running scripts | after P7 (same graph surgery) |
| P10 | Raw nu builder (skip stdenv phases) | ~200-400ms x 264 drvs of CPU | deferred: conflicts with stdenv-based crateOverrides; revisit |
| P11 | Remote builders across the colmena fleet + cachix | cold builds beat monolithic cargo outright | infra config, not library code; user action |
| P12 | Cross-compilation: dual-platform resolution (host edges vs target edges), `--target` compiles, host proc-macros/build scripts, pkgsCross cc for linking | capability, not speed | minimal version done: `crossTarget`/`crossCC` params; wclip cross-built to aarch64 (build script host-compiled, libc target-compiled) and RAN under qemu binfmt. Known limit: crates needed by both worlds get separate host variants; proc-macro-heavy graphs untested |
| P13 | Within-crate incremental compilation | out of reach: rustc incr cache is stateful/nondeterministic, irreconcilable with hermetic builds; cranelift compensates | rejected |

## Decision log

- 2026-07-18 (evening): wild is the default linker for every
  buildCargoProject call (injected in the perSystemLib wiring; override
  with `linker = null` or another package). rust-systemd additionally has
  a `rust-systemd-dev` variant: debug profile, cranelift codegen, wild
  linking, with `-Clinker-features=-lld` to opt out of nightly's rust-lld
  default which would bypass the -B linker shim.
- 2026-07-18: Library lives at `nix/lib/cargo/`, sibling of `nix/lib/deno`;
  exposed via `perSystemLib` like the deno lib. (Initially scaffolded at
  `nix/cargo/`, moved on user correction.)
- 2026-07-18: Snapshot mini-index is the default sourcing strategy; full
  index input supported but not wired in (repo weight, flake.lock churn).
- 2026-07-18: `lib/` is builtins-only for portability and cheap tests.
- 2026-07-18: Stage feature resolution v1-unified first with `cargo metadata`
  as the oracle, resolver v2 as M8 rather than blocking first builds.
