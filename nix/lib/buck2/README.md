# nix-buck2

A Nix builder for [Buck2](https://github.com/facebook/buck2) projects. It
parses a project's `.buckconfig` and its `BUCK`/`.bzl` Starlark files at
evaluation time and lowers each Buck2 *action* to its own Nix derivation, with
no import-from-derivation and no `buck2` binary in the loop. Built on the
reusable Starlark interpreter in [`../skylark`](../skylark); design,
milestones, and the value/effect model are in [PLAN.md](./PLAN.md).

Status: the `examples/no_prelude` C++ binary, C++ shared library, and Rust
binary build (one derivation per action) as flake checks. The Go vertical does
not build yet (it needs a `buck-out`-relative artifact model for
`cmd_args(relative_to = ...)`), and `select()` / configuration are stubbed. See
PLAN.md.

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
3. **Lowering.** Each action becomes one derivation: `ctx.actions.run` a
   `runCommand` with the toolchain on `PATH`, `ctx.actions.write` a text file,
   `ctx.actions.download_file` a `fetchurl` fixed-output derivation (the sha256
   is in the Starlark source, so it stays pure). Source artifacts enter the
   sandbox via `builtins.path` (staged at their package-relative path so
   `#include`s resolve); dependency outputs are wired through store-path
   interpolation. A target's default output is its `DefaultInfo` default output
   lowered to the producing action's derivation.

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

## Files

- `default.nix` flakelight module: `buildBuck2Project` and `buck2Lib` via
  `perSystemLib`
- `lib/` pure-eval phases (builtins only): `buckconfig`, `labels`, `loader`,
  `globals`, `attrs`/coercion + `analysis`, `actions`, `cmd_args`
- `build/` lowering (`lower.nix`), toolchain map (`toolchains.nix`), the
  `buildBuck2Project` entry point
- `tests/` eval unit tests and the committed `no_prelude` fixture
- `checks.nix` flake checks (build individually; never `nix flake check`)
