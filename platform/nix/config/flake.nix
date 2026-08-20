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
    # rust-overlay is deliberately NOT declared: only zed's wasm toolchain
    # uses rust-bin, so the tree that wants it declares it (the monorepo
    # root does) and everyone else fetches nothing. git-hooks comes
    # transitively as workspace/git-hooks - the framework pins it anyway,
    # and a second declaration here was one more input for every consumer
    # to follow. Flakes have no optional inputs of their own: NixOS/nix#7205.
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable";

    # For `outputsIn`, which turns this directory into a module, and for
    # nixpkgs, which everything below follows out of it rather than off a
    # second declaration here. One input is one thing for a consuming flake
    # to follow, and there is no arrangement of it that leaves the framework
    # and this directory on different nixpkgs.
    workspace.url = "git+https://tangled.org/overby.me/nix-workspace";
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
