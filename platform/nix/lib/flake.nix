# Build systems, lowered to Nix.
#
# Pure-eval builders for Cargo, Buck2, Ninja, Deno and Starlark. Each reads the
# upstream build description at evaluation time and emits one derivation per
# build unit, so every action caches independently, Nix schedules the DAG, and
# editing one source rebuilds only what depends on it.
#
# A workspace of its own because thirty-eight things build with these and none
# of them is a machine configuration.
#
# `lib/<name>.nix` becomes `lib.<name>`, which is why the directory is named
# again inside: this flake is the workspace, `lib/` is the output it feeds.
{
  description = "Pure-eval Nix builders for Cargo, Buck2, Ninja, Deno and Starlark";

  inputs = {
    # nixpkgs comes through this rather than beside it: one input is one thing
    # for a consuming flake to follow, and no arrangement of it leaves the two
    # disagreeing.
    workspace.url = "git+https://tangled.org/overby.me/nix-workspace";

    # Not a fixture: `buildNinjaProject` extracts its graph by running
    # `oxidized-ninja -t graph-json`, a subcommand of the rewrite. There is no
    # nixpkgs ninja to fall back to.
    oxidized-ninja = {
      url = "git+https://tangled.org/overby.me/oxidized-ninja";
      inputs.workspace.follows = "workspace";
    };
  };

  outputs = inputs:
    inputs.workspace {
      inherit inputs;
      outputDirs = [./.];
    };
}
