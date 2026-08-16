# Monorepo example workspace using nix-workspace
#
# This flake demonstrates a multi-subworkspace monorepo setup.
# The root workspace defines shared packages, and two subworkspaces
# (lib-a and app-b) each have their own workspace.ncl with independent
# packages that get automatically namespaced in the flake outputs.
#
# Output structure:
#   packages.<system>.shared-lib     — from root workspace
#   packages.<system>.lib-a          — from lib-a/packages/default.ncl
#   packages.<system>.app-b          — from app-b/packages/default.ncl
#   packages.<system>.app-b-cli      — from app-b/packages/cli.ncl
#   devShells.<system>.default       — from root workspace
#
# Usage:
#   nix build .#shared-lib
#   nix build .#lib-a
#   nix build .#app-b
#   nix build .#app-b-cli
#   nix develop
#
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nix-workspace.url = "git+https://tangled.org/@overby.me/overby.me?dir=nix-workspace";
  };

  outputs = inputs:
    inputs.nix-workspace ./. {
      inherit inputs;
    };
}
