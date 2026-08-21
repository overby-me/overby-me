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
    #
    # The host upstreams are set through this input rather than declared
    # beside the others, so this one flake still evaluates its hosts
    # standalone while naming each pin in one place. The framework declares
    # them as optional inputs defaulting to a stub; overriding one is what
    # enables it, and a consumer that follows this flake's `workspace` gets
    # the same upstreams without restating any of them.
    #
    # secretspec points back at the framework's own repo: upstream has no
    # flake, so it carries a wrapper that holds the pin.
    workspace = {
      url = "git+https://tangled.org/overby.me/nix-workspace";
      inputs = {
        secretspec.url = "git+https://tangled.org/overby.me/nix-workspace?dir=upstreams/secretspec";

        home-manager.url = "github:nix-community/home-manager/release-26.05";
        nix-darwin.url = "github:nix-darwin/nix-darwin/nix-darwin-26.05";
        # Narrow-window upstream: newer revisions drop the nix.* module the
        # system-modules set. Both pins match what the monorepo proved.
        system-manager = {
          url = "github:numtide/system-manager/48d47346e0c6ad05b6c869ea92649c47723d1cfc";
          inputs.nixpkgs.url = "github:NixOS/nixpkgs/61b7c44c4073f0b827768aff0049561b5110ea5a";
        };
      };
    };

    # The remaining upstreams: direct, not integrations. An input exporting
    # a default package lands in pkgs under its own name, and hosts import
    # nixos-hardware's modules by name, so declaring one is the whole of
    # having it.

    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    nixos-hardware.url = "github:NixOS/nixos-hardware/master";
    zen-browser = {
      # Pinned as in the monorepo root: zen tracks nixpkgs-unstable at HEAD.
      url = "github:0xc000022070/zen-browser-flake/945efbc704b7f8c1731a922aabbc5d95edc9eb74";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
      inputs.home-manager.follows = "workspace/home-manager";
    };
    nix-wallpaper = {
      url = "github:lunik1/nix-wallpaper";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
      inputs.pre-commit-hooks.follows = "workspace/git-hooks";
    };
    # Published projects whose modules and packages the hosts consume; each
    # repo exports its own module beside its build.
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
    # Deliberately absent: rust-overlay - zed degrades to no wasm toolchain
    # without it, which a standalone host eval can live with.
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
      outputDirs = [./.];
    };
}
