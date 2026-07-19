# nix-buck2

A Nix builder for [Buck2](https://github.com/facebook/buck2) projects. It
parses a project's `.buckconfig` and its `BUCK`/`.bzl` Starlark files at
evaluation time and lowers each Buck2 *action* to its own Nix derivation, with
no import-from-derivation and no `buck2` binary in the loop. Built on the
reusable Starlark interpreter in [`../skylark`](../skylark); design,
milestones, and the value/effect model are in [PLAN.md](./PLAN.md).

Status: all four `examples/no_prelude` build targets build (one derivation per
action) and run as flake checks: the C++ binary, the C++ shared library, the
Rust binary, and the Go binary (whose toolchain is downloaded, unpacked, and
symlinked entirely through Buck2 actions). `select()` / configuration and
`//...` target discovery are not implemented yet. See PLAN.md.

## How it works

1. **Load.** `.buckconfig` `[cells]` are parsed; each needed `BUCK` file is
   evaluated with the skylark interpreter under a target-registry `world`, so a
   rule call like `cpp_binary(name = "main", ...)` registers a target node.
   `load()` pulls in `.bzl` modules (rule and provider definitions); `glob()`
   reads the package directory with `builtins.readDir`. The result is an
   unconfigured target graph, as plain Nix data.
2. **Analysis.** Each target's attrs are coerced against the rule's `attrs`
   schema (`attrs.source()` to a source artifact, `attrs.dep()` /
   `attrs.toolchain_dep()` to an analyzed dependency's providers, indexable by
   provider type as `dep[SomeInfo]`). A `ctx` is built and the rule's `impl` is
   run through the interpreter; `ctx.actions.run/write/download_file` register
   actions into a threaded action registry and mint artifacts. The impl returns
   providers (`DefaultInfo`, `RunInfo`, ...). Still plain Nix data, still no
   IFD.
3. **Lowering.** Each action becomes one derivation over a virtual `buck-out`:
   every artifact has a stable working-directory-relative path, and command
   lines / generated scripts reference those relative paths (honoring
   `cmd_args(relative_to = ...)`), not store paths, so an action that only needs
   a peer's path (a script naming an output it does not build) creates no
   dependency and no cycle. Each derivation stages its inputs into a working
   tree (sources via `builtins.path`; a producer's whole tree copied in, so
   transitive files and symlink targets travel along), runs, and exports the
   tree as `$out`. `ctx.actions.run` is a `runCommand` with the toolchain on
   `PATH`, `write` a text file, `download_file` a `fetchurl` fixed-output
   derivation (sha256 from the source, so it stays pure); downloaded prebuilt
   binaries are made runnable with `autoPatchelfHook`. A target's default output
   is its `DefaultInfo` default output in the producing action's tree.

The upstream "local" toolchains (`command = "clang++"` / `"rustc"`) are made
hermetic by mapping the command string to a nixpkgs package
(`build/toolchains.nix`), overridable via `toolchainPackages`.

## Usage

In a flakelight package definition:

```nix
packages.hello = {lib, ...}:
  lib.buildBuck2Project {
    src = ./examples/no_prelude;      # contains .buckconfig, BUCK files, sources
    target = "//cpp/hello_world:main";
  };
```

The result's `$out` holds the target's default output (e.g. `result/main`).

### Parameters

| Parameter | Default | Description |
|---|---|---|
| `src` | required | Project root (contains `.buckconfig`) |
| `target` | `null` | A single target label to build |
| `targets` | `null` | A list of labels (produces a `symlinkJoin`) |
| `system` | `pkgs`'s system | Platform key feeding `host_info()` and toolchains |
| `toolchainPackages` | clang/rustc/go map | Toolchain command name to nixpkgs package |
| `ifdAnalysis` | `false` | Run load+analysis in a cached derivation (one IFD) instead of at eval time; see below |

### `ifdAnalysis` (experimental)

By default, analysis (parsing the Starlark and running the rule impls) happens
during Nix evaluation on every build. With `ifdAnalysis = true`, it runs once
inside a derivation that emits the action graph as JSON, which Nix then imports
(one import-from-derivation). That derivation is keyed on the build files
(`.bzl` / `BUCK` / `.buckconfig`) and the file-name structure only, not source
contents, so editing a source never re-runs the interpreter and a no-op rebuild
reuses the cached graph.

This only pays off when interpretation is expensive (large, prelude-based
projects). On small projects the eval-time cost of building the content-keyed
analysis source plus the JSON round-trip exceeds the interpreter cost, so it is
measurably slower there; it stays opt-in and off by default.

`lib.buck2Lib` exposes the pure phases (`buckconfig`, `labels`, `loader`,
`analysis`, `globals`, `actions`, `cmd_args`) and the `skylark` interpreter for
tests and advanced use.

## Tests

- Eval unit tests: label/cell resolution, the load-phase target graph, and the
  analysis-phase action graph, over the committed `no_prelude` fixtures.

  ```console
  nix eval -f nix/lib/buck2/tests/labels.nix
  nix eval -f nix/lib/buck2/tests/load.nix
  nix eval -f nix/lib/buck2/tests/analysis.nix
  ```

- End-to-end flake checks (build the binary, run it, assert output):
  `nix build .#checks.x86_64-linux.buck2-build-cpp` and `...buck2-build-rust`.
  Never `nix flake check` in this repo.

## Performance

On the trivial `no_prelude` targets, upstream buck2 is 4-60x faster: it keeps a
warm daemon (hot rebuild ~10 ms) while nix-buck2 re-evaluates load+analysis+
lowering on every invocation (~0.6 s) and pays per-derivation setup plus cold
compiler caches (Go's GOCACHE recompiles the stdlib each sandboxed build). The
clang/rustc compile itself (~1.4 s, dominated by `#include <iostream>`) is paid
by both. nix-buck2's advantages are structural rather than latency: global
content-addressed caching shared across projects and machines, hermetic pinned
toolchains, and Nix-native builds (a downloaded toolchain runs even in the pure
sandbox via autoPatchelf). Dependency trees are staged by symlink so a large
toolchain is never copied per consumer. Full numbers, the wall-time breakdown,
the structural analysis, and the optimization notes are in [PLAN.md](./PLAN.md).

## Files

- `default.nix` flakelight module: `buildBuck2Project` and `buck2Lib` via
  `perSystemLib`
- `lib/` pure-eval phases (builtins only): `buckconfig`, `labels`, `loader`,
  `globals`, `attrs`/coercion + `analysis`, `actions`, `cmd_args`
- `build/` lowering (`lower.nix`), toolchain map (`toolchains.nix`), the
  `buildBuck2Project` entry point
- `tests/` eval unit tests and the committed `no_prelude` fixture
- `checks.nix` flake checks (build individually; never `nix flake check`)
