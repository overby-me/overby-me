# Base system-manager configuration for a generic Ubuntu/Debian host, and the
# Linux counterpart to the `darwitas` darwin config: a reusable base other
# flakes extend rather than a config for one machine.
#
#     nix run 'github:numtide/system-manager' -- switch --flake .#ubuntas
#
# Returns the arguments for `system-manager.lib.makeSystemConfig`, which does
# not pass flake inputs to modules - hence `specialArgs`.
#
# Downstream flakes compose `inputs.self.systemModules.core` with their own
# host-specific modules in their own `systemConfigs.<host>`; the downstream
# template's `system-manager/config/workhost.nix` is that pattern.
{inputs, ...}: {
  specialArgs = {inherit inputs;};

  modules = [
    inputs.self.systemModules.core

    {
      # system-manager builds its own `pkgs` from this, so it is what wires the
      # whole configuration to x86_64-linux.
      nixpkgs.hostPlatform = "x86_64-linux";

      # Defaults to false in system-manager, and with it off, activating a
      # generation *removes* the nix-daemon units it previously managed,
      # tearing down the daemon.
      nix.enable = true;

      # Officially Ubuntu and Debian; this opts in to close derivatives.
      system-manager.allowAnyDistro = true;
    }
  ];
}
