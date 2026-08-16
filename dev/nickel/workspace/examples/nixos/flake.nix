# NixOS machine configuration example using nix-workspace
#
# This flake demonstrates how nix-workspace manages NixOS machine
# configurations, NixOS modules, and auto-discovery from convention
# directories.
#
# Outputs:
#   nixosConfigurations.my-machine  — A NixOS system configuration
#   nixosModules.desktop            — A reusable NixOS module
#
# Usage:
#   nixos-rebuild switch --flake .#my-machine
#   nix build .#nixosConfigurations.my-machine.config.system.build.toplevel
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
