# nix-deno

Nix builder for Deno projects with npm dependencies. Parses `deno.lock` at evaluation time to create individual `fetchurl` derivations per npm package, so no manual hash maintenance is needed.

## How it works

1. `fetchDenoDeps.nix` reads `deno.lock` via `builtins.fromJSON`, extracts every npm entry, and fetches each tarball with `fetchurl` using the integrity hash from the lock file.
2. The fetched tarballs are assembled into a DENO_DIR-compatible cache layout (`npm/registry.npmjs.org/<package>/<version>/`) with generated `registry.json` metadata files.
3. `buildDenoProject.nix` uses this cache to run `deno install --frozen` in the Nix sandbox, then executes the provided build command.

## Usage

`buildDenoProject` is exposed as `lib.buildDenoProject` in `pkgs` via the `perSystemLib` flakelight module.

In a flakelight package definition:

```nix
packages.my-app = { lib, ... }:
  lib.buildDenoProject {
    pname = "my-app";
    src = ./.;
    buildCommand = "deno run -A npm:@rsbuild/core build";
    installPhase = "cp -r dist $out";
  };
```

### Parameters

| Parameter | Default | Description |
|---|---|---|
| `src` | required | Source directory containing `deno.lock` |
| `buildCommand` | required | Shell command to build the project |
| `installPhase` | required | Shell commands to install build output to `$out` |
| `pname` | `"deno-project"` | Package name |
| `version` | `"0.0.0"` | Package version |
| `lockFile` | `src + "/deno.lock"` | Path to the Deno lock file |
| `nativeBuildInputs` | `[]` | Additional build inputs |
| `meta` | `{}` | Nixpkgs meta attributes |

## Files

- `default.nix` — Flakelight module exposing `buildDenoProject` via `perSystemLib`
- `buildDenoProject.nix` — Project builder derivation
- `fetchDenoDeps.nix` — npm dependency fetcher using dynamic derivations from `deno.lock`
