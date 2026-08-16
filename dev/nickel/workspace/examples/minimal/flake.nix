# Minimal example workspace using nix-workspace
#
# This flake demonstrates the simplest possible nix-workspace setup:
# a single package discovered from the packages/ directory and a
# development shell discovered from shells/.
#
# Usage:
#   nix build .#hello
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
