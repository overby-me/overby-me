# Build systems, lowered to Nix.
#
# Pure-eval builders for Cargo, Buck2, Ninja, Deno and Starlark: each reads
# the upstream build description at evaluation time and emits one derivation
# per build unit, so every action is cached independently, Nix schedules the
# DAG, and editing one source rebuilds only what depends on it.
#
# A workspace of its own because of who uses it. Thirty-eight things in the
# monorepo build with these - every oxidized port, the media crates, wclip,
# fe-c - and not one of them is a machine configuration. Reaching
# `buildCargoProject` through a flake that also carries eight NixOS hosts and
# a secrets directory had it backwards, and it was most of that flake: 1482 of
# its 1687 files were this, and 1361 of those are the committed registry
# index.
#
# `lib/<name>.nix` becomes `lib.<name>`, so the directory is named again
# inside: this flake is the workspace, and `lib/` is the output it feeds.
{
  description = "Pure-eval Nix builders for Cargo, Buck2, Ninja, Deno and Starlark";

  inputs = {
    # For `outputsIn`, and for nixpkgs, which comes through it rather than
    # being declared beside it: one input is one thing for a consuming flake
    # to follow, and no arrangement of it leaves the two disagreeing.
    workspace.url = "git+https://tangled.org/overby.me/nix-workspace";

    # Real Cargo workspaces, which the cargo library is tested against and the
    # ninja library builds its graph extractor from. They were sibling
    # directories reached by counting `..`, which is a thing a flake cannot do:
    # its source is its own directory, so those paths worked only because the
    # monorepo happened to be what surrounded them.
    #
    # Fixtures rather than vendored copies on purpose: what the cargo tests
    # catch is the resolver meeting a manifest feature it does not handle,
    # which a snapshot stops doing the day after it is taken. The tree above
    # overrides them onto its own copies, so a change to a port is what the
    # check builds rather than whatever was last pushed.
    wclip = {
      url = "git+https://tangled.org/overby.me/wclip";
      inputs.workspace.follows = "workspace";
    };
    oxidized-xz = {
      url = "git+https://tangled.org/overby.me/oxidized-xz";
      inputs.workspace.follows = "workspace";
    };

    # Not a fixture: `buildNinjaProject` extracts a build graph by running
    # `oxidized-ninja -t graph-json`, and `graph-json` is a subcommand of the
    # rewrite. There is no nixpkgs ninja to fall back to.
    oxidized-ninja = {
      url = "git+https://tangled.org/overby.me/oxidized-ninja";
      inputs.workspace.follows = "workspace";
    };
  };

  # A module, like the workspaces beside it: a tree that has this input has
  # the builders, as `pkgs.lib.<name>` for the ones that need a pkgs and as
  # the flake's own `lib` output for the ones that do not.
  outputs = inputs: let
    inherit (inputs.workspace.inputs) nixpkgs;
    inherit (nixpkgs) lib;

    own =
      map (n: ./workspace-modules + "/${n}")
      (builtins.attrNames (builtins.readDir ./workspace-modules));
  in {
    workspaceModule = {
      imports = [(inputs.workspace.outputsIn ./.)] ++ own;

      # nixpkgs is put back in by name: it is not an input of this flake any
      # more, and a module that reads `inputs.nixpkgs` would otherwise get
      # whatever the consuming tree happens to call nixpkgs.
      inputs =
        lib.mapAttrs (_: lib.mkDefault)
        ((removeAttrs inputs ["self"]) // {inherit nixpkgs;});

      outputDirs = [./.];
    };
  };
}
