# nix-ninja

A Nix builder for [Ninja](https://ninja-build.org/) projects. It extracts a
configured project's build graph with `rust-ninja -t graph-json` (one
import-from-derivation) and lowers each Ninja *edge* to its own Nix derivation,
with no `ninja` binary scheduling the build. A sibling to
[`../buck2`](../buck2) (per-action Buck2 builds) and [`../cargo`](../cargo)
(per-crate builds); design and milestones are in [PLAN.md](./PLAN.md).

Status: the hand-written `trivial` fixture (compile one C file, link an
executable) builds one derivation per edge and runs as a flake check. Header
dependencies (`deps = gcc`/`depfile`), CMake-generated manifests, and scale are
the next milestones. See PLAN.md.

## Why per-edge derivations

`ninja` runs one process that owns the whole build graph: no per-target Nix
caching, no parallelism Nix can see, a full rebuild on any input change.
Lowering each Ninja edge to a derivation gives the same value as the cargo and
buck2 libraries:

- every edge (compile, link, codegen) is cached independently in the Nix store
  (and Cachix), shared across machines,
- Nix schedules the edge DAG across cores and remote builders,
- editing one source rebuilds only its edge and that edge's dependents — a fast
  incremental loop over an unmodified CMake/Meson/gn build.

## How it works

1. **Extract.** `rust-ninja -t graph-json` parses `build.ninja` (and its
   `subninja`/`include`s) and emits every edge as JSON with its
   fully-expanded command and all input/output/dep/depfile/rspfile paths. This
   runs once in a derivation; Nix reads the JSON (the single IFD). `rust-ninja`
   already does the hard parsing and `$in`/`$out`/variable expansion; the
   `graph-json` tool is a thin dumper over its `State`.
2. **Lower.** Each edge becomes one derivation over a virtual build tree rooted
   at the Ninja build directory (`build/lower.nix`, mirroring
   `../buck2/build/lower.nix`). A producer edge's whole `$out` tree is
   symlinked in (`cp -rs`, so transitive files travel and large trees are never
   copied); source inputs are copied as real files so `#include` resolves; the
   expanded command runs against build-dir-relative paths and the result tree is
   `$out`. Dependencies flow through store-path interpolation in the staging
   commands only.

## Usage

In a flakelight package or check:

```nix
lib.buildNinjaProject {
  src = ./path/to/configured-build-dir;   # contains build.ninja
  target = "bin/hello";                    # a build output path (or omit for `default`)
}
```

The result's `$out/<target>` is the built output.

### Parameters

| Parameter | Default | Description |
|---|---|---|
| `src` | required | Directory containing `build.ninja` and the reachable sources |
| `target` | `null` | A single output path to build |
| `targets` | `null` | A list of output paths (produces a `symlinkJoin`) |
| `ninjaFile` | `"build.ninja"` | Manifest filename |
| `toolchain` | `[stdenv.cc coreutils]` | Packages on `PATH` for every edge command |
| `rustNinja` | built here | The `rust-ninja` package used for graph extraction |

`lib.ninjaLib` exposes the lowering phase for tests and advanced use.

## Tests

End-to-end flake check (build the binary, run it, assert output):

```console
nix build .#checks.x86_64-linux.ninja-build-trivial
```

(Never `nix flake check` in this repo.)
