# What this tree's nix configuration talks to.
#
# The modules taken out of nix-workspace, declared here rather than in the
# root flake. The framework itself is not among them: the root calls it, and
# nothing in this directory talks to it.
#
# Each module is a flake owning one upstream and the module that uses it, so a
# tree takes only what it talks to and the name says what taking it costs -
# and this is the directory doing the talking.
#
# It only carries inputs. A flake's own source is its directory, so this one
# cannot see safety/ or apps/, and discovery over the whole tree stays with
# the root flake, which reaches these through `inputs` on this input.
#
# nixpkgs comes through the framework rather than being declared here: without
# that a module builds its upstream against a nixpkgs of its own, which is how
# zen-browser came to want an ffmpeg the rest of the tree does not have. It was
# declared here and followed onto the root's, which worked as long as every
# consumer remembered to follow it - one input to follow cannot be half done.
{
  description = "The framework and the modules this tree's nix configuration uses";

  inputs = {
    # Read by the overlays in this directory, and by nothing above it.
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };

    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };

    # For `outputsIn`, which turns this directory into a module, and for
    # nixpkgs, which everything below follows out of it rather than off a
    # second declaration here. One input is one thing for a consuming flake
    # to follow, and there is no arrangement of it that leaves the framework
    # and this directory on different nixpkgs.
    workspace = {
      url = "git+https://tangled.org/overby.me/nix-workspace";
      inputs.git-hooks.follows = "git-hooks";
    };

    workspace-darwin = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/darwin";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    workspace-disko = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/disko";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    workspace-home-manager = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/home-manager";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    workspace-nixos-hardware = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nixos-hardware";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    workspace-nixos-raspberrypi = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nixos-raspberrypi";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    workspace-nix-wallpaper = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/nix-wallpaper";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    workspace-ragenix = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/ragenix";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    workspace-system-manager = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/system-manager";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    workspace-zen-browser = {
      url = "git+https://tangled.org/overby.me/nix-workspace?dir=modules/zen-browser";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
  };

  # A module, so this directory is a workspace rather than a bag of inputs:
  # it carries the outputs under it and the modules it takes, and a tree that
  # has this input gets both by having it.
  #
  # A module rather than outputs of its own, because outputs would be a second
  # evaluation, and the projects elsewhere in the tree build with a `lib`
  # defined in another workspace - a lib in another flake's evaluation is not
  # one they can reach.
  outputs = inputs: {workspaceModule = inputs.workspace.workspaceIn ./. inputs;};
}
