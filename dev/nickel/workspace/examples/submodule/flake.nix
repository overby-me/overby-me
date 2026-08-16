# Git submodule example workspace using nix-workspace
#
# This flake demonstrates how nix-workspace handles git submodules
# (or any external checkout) as subworkspaces. The external-tool/
# directory simulates a git submodule that contains its own workspace.ncl.
#
# In a real setup, you would add the submodule with:
#   git submodule add <remote-url> external-tool
#
# And use submodules in the flake URL if needed:
#   git+file:.?submodules=1
#
# Output structure:
#   packages.<system>.my-tool           — from root workspace
#   packages.<system>.external-tool     — from external-tool/packages/default.ncl
#   packages.<system>.external-tool-lib — from external-tool/packages/lib.ncl
#   devShells.<system>.default          — from root workspace
#
# Usage:
#   nix build .#my-tool
#   nix build .#external-tool
#   nix build .#external-tool-lib
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
