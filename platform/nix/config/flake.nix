# What this tree's nix configuration talks to.
#
# The workspace-* integrations are NOT declared here. An integration is
# enabled by whoever declares it, and hosts force their modules lazily - so
# the tree that evaluates a host declares the integrations that host needs
# (the monorepo root does), and a consumer that takes this repo for its
# users, modules and overlays inherits no pin it does not name. Project
# discovery stays with the root flake: a flake's source is its own
# directory, so this one cannot see safety/ or apps/.
#
# nixpkgs comes through the framework rather than being declared here. Declared
# separately it has to be followed separately, and a consumer that forgets
# builds a module's upstream against a nixpkgs of its own - which is how
# zen-browser came to want an ffmpeg the rest of the tree does not have.
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
  };

  # A module, so this directory is a workspace rather than a bag of inputs:
  # it carries the outputs under it and the modules it takes, and a tree that
  # has this input gets both by having it.
  #
  # A module rather than outputs of its own, because outputs would be a second
  # evaluation, and the projects elsewhere in the tree build with a `lib`
  # defined in another workspace - a lib in another flake's evaluation is not
  # one they can reach.
  outputs = inputs:
    inputs.workspace {
      inherit inputs;
      module = true;
    };
}
