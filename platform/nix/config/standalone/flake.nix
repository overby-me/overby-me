# nix-config, standalone: every host evaluable from this directory alone.
#
# The module flake one level up stays thin - a consumer taking it inherits
# no pin it does not name, and input fetching is lazy besides (measured: an
# unforced input is never downloaded). This flake is the other face: it
# declares every upstream the hosts force, so
#
#   nix build 'git+https://tangled.org/overby.me/nix-config?dir=standalone#nixosConfigurations.<host>...'
#
# works from a clone with nothing else. Two flakes rather than one union,
# because the union recurses: self sits among the inputs, and forcing the
# union's names forces the tree evaluation those names come from.
{
  description = "nix-config with every host upstream declared";

  inputs = {
    workspace.url = "git+https://tangled.org/overby.me/nix-workspace";
    nix-config = {
      url = "path:..";
      inputs.workspace.follows = "workspace";
    };

    home-manager = {
      url = "github:nix-community/home-manager/release-26.05";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    nix-darwin = {
      url = "github:nix-darwin/nix-darwin/nix-darwin-26.05";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    secretspec = {
      url = "github:cachix/secretspec/v0.19.1";
      flake = false;
    };
    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
    };
    nixos-hardware.url = "github:NixOS/nixos-hardware/master";
    zen-browser = {
      # Pinned as in the monorepo root: zen tracks nixpkgs-unstable at HEAD.
      url = "github:0xc000022070/zen-browser-flake/945efbc704b7f8c1731a922aabbc5d95edc9eb74";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
      inputs.home-manager.follows = "home-manager";
    };
    nix-wallpaper = {
      url = "github:lunik1/nix-wallpaper";
      inputs.nixpkgs.follows = "workspace/nixpkgs";
      inputs.pre-commit-hooks.follows = "workspace/git-hooks";
    };
    # Narrow-window upstream: newer revisions drop the nix.* module the
    # system-modules set. Both pins match what the monorepo proved.
    system-manager = {
      url = "github:numtide/system-manager/48d47346e0c6ad05b6c869ea92649c47723d1cfc";
      inputs.nixpkgs.url = "github:NixOS/nixpkgs/61b7c44c4073f0b827768aff0049561b5110ea5a";
      inputs.userborn.inputs.pre-commit-hooks-nix.follows = "workspace/git-hooks";
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

  outputs = inputs:
    inputs.workspace {
      inherit inputs;
      systems = ["x86_64-linux" "aarch64-linux" "aarch64-darwin"];
      outputDirs = [./.];
    };
}
