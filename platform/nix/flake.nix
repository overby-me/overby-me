# What this tree's nix configuration talks to.
#
# nix-workspace and the modules taken out of it, declared here rather than in
# the root flake. Each module is a flake owning one upstream and the module
# that uses it, so a tree takes only what it talks to and the name says what
# taking it costs - and this is the directory doing the talking.
#
# It only carries inputs. A flake's own source is its directory, so this one
# cannot see safety/ or apps/, and discovery over the whole tree stays with
# the root flake, which reaches these through `inputs` on this input.
#
# nixpkgs comes from the root rather than being pinned twice: without that a
# module builds its upstream against a nixpkgs of its own, which is how
# zen-browser came to want an ffmpeg the rest of the tree does not have.
{
  description = "The framework and the modules this tree's nix configuration uses";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    # Read by the overlays in this directory, and by nothing above it.
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    workspace = {
      url = "git+https://tangled.org/overby.me/nix-workspace";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        git-hooks.follows = "git-hooks";
      };
    };

    workspace-darwin = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/darwin";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-disko = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-drowse = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/drowse";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-home-manager = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-modular-skills = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/modular-skills";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-nixos-hardware = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nixos-hardware";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-nixos-raspberrypi = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nixos-raspberrypi";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-nix-wallpaper = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nix-wallpaper";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-ragenix = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/ragenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-system-manager = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/system-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    workspace-zen-browser = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/zen-browser";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  # Nothing but the inputs: the root flake merges these into its own so the
  # workspace finds every module, and calls the framework from here.
  outputs = inputs: {
    modules = removeAttrs inputs ["self"];
  };
}
