# Example: Rust project using the nix-workspace-rust plugin
#
# This demonstrates how plugins extend nix-workspace with
# language-specific conventions, contracts, and builders.
#
# The nix-workspace-rust plugin adds:
#   - A `crates/` convention directory (auto-discovered as Rust packages)
#   - Extended PackageConfig fields (edition, features, cargo-lock, etc.)
#   - Enhanced Rust builder with feature flag and workspace member support
#
# Directory structure:
#   rust-project/
#   ├── flake.nix          # This file
#   ├── workspace.ncl      # Workspace config with plugins = ["nix-workspace-rust"]
#   ├── packages/
#   │   └── my-tool.ncl    # A generic package (standard convention)
#   └── crates/
#       └── my-lib.ncl     # A Rust crate (plugin convention → packages output)
#
# Resulting flake outputs:
#   packages.<system>.my-tool     — from packages/ (generic builder)
#   packages.<system>.my-lib      — from crates/ (rust builder via plugin)
#   devShells.<system>.default    — auto-generated default shell
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nix-workspace.url = "git+https://tangled.org/@overby.me/overby.me?dir=nix-workspace";
  };

  outputs = inputs:
    inputs.nix-workspace ./. {
      inherit inputs;
      # Plugins can also be specified here on the Nix side,
      # in addition to (or instead of) workspace.ncl.
      plugins = ["nix-workspace-rust"];
    };
}
