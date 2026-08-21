# What this tree's nix configuration talks to.
#
# nixpkgs comes through the framework, not declared here: a separate
# declaration has to be followed separately, and a consumer that forgets gets
# an upstream built against a nixpkgs of its own.
#
# Project discovery stays with the root flake, which can see safety/ and apps/;
# this directory cannot.
{
  description = "The framework and the modules this tree's nix configuration uses";

  inputs = {
    # Read by the overlays here, and by nothing above. rust-overlay is
    # deliberately absent: only zed's wasm toolchain wants it.
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable";

    # For `outputsIn` and for nixpkgs, which everything below follows out of.
    # The host upstreams are optional inputs on it, set here so this flake
    # still evaluates its hosts standalone and a consumer restates no pin.
    workspace = {
      url = "git+https://tangled.org/overby.me/nix-workspace";
      inputs = {
        # Upstream ships no flake, so the framework carries one holding the pin.
        secretspec.url = "git+https://tangled.org/overby.me/nix-workspace?dir=upstreams/secretspec";

        home-manager.url = "github:nix-community/home-manager/release-26.05";
        nix-darwin.url = "github:nix-darwin/nix-darwin/nix-darwin-26.05";
        system-manager.url = "github:numtide/system-manager";
      };
    };

    # Direct upstreams, not integrations: declaring one is the whole of it.
    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    nixos-hardware.url = "github:NixOS/nixos-hardware/master";
    zen-browser = {
      # Pinned as in the monorepo root: HEAD wants ffmpeg_9, absent from 26.05.
      url = "github:0xc000022070/zen-browser-flake/945efbc704b7f8c1731a922aabbc5d95edc9eb74";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
      inputs.home-manager.follows = "workspace/home-manager";
    };
    nix-wallpaper = {
      url = "github:lunik1/nix-wallpaper";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
      inputs.pre-commit-hooks.follows = "workspace/git-hooks";
    };
    # Published projects whose modules and packages the hosts consume.
    tangled-spindle-nix-engine = {
      url = "git+https://tangled.org/overby.me/tangled-spindle-nix-engine";
      inputs.workspace.follows = "workspace";
    };
    nushell-plugin-tramp = {
      url = "git+https://tangled.org/overby.me/nushell-plugin-tramp";
      inputs.workspace.follows = "workspace";
    };
    # The package collection the desktops and home modules draw from.
    nix-packages = {
      url = "git+https://tangled.org/overby.me/nix-packages";
      inputs.workspace.follows = "workspace";
    };
  };

  # A module rather than outputs of its own: outputs would be a second
  # evaluation, and a lib defined there is not one the rest of the tree can
  # reach.
  outputs = inputs:
    inputs.workspace {
      inherit inputs;
      module = true;
      outputDirs = [./.];
    };
}
