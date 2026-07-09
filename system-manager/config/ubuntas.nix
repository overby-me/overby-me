# Base system-manager configuration for a generic Ubuntu/Debian host.
#
# `ubuntas` is the Linux counterpart to the `darwitas` darwin config: a reusable
# base that other flakes extend, rather than a config tied to one machine. It
# applies NixOS-style modules to a non-NixOS Linux distro using numtide's
# system-manager (https://github.com/numtide/system-manager) — managing a subset
# of the system: packages in the system profile, systemd services, and files
# under /etc. Apply it with:
#
#     nix run 'github:numtide/system-manager' -- switch --flake .#ubuntas
#
# This file is autoloaded as `systemConfigs.ubuntas` by the flakelight module in
# `flake/modules/systemConfigs.nix` (via `nixDirAliases`), mirroring how
# `darwin/config` and `home-manager/config` autoload. It returns the arguments
# for `system-manager.lib.makeSystemConfig` — the module builds it. system-manager
# instantiates its own `pkgs` from `nixpkgs` based on `nixpkgs.hostPlatform`, so
# modules receive a `pkgs` arg the usual way.
#
# `makeSystemConfig` does not pass flake `inputs` to modules, so we forward them
# via `specialArgs`.
#
# Downstream flakes reuse this base by composing `inputs.self.systemModules.core`
# (the generic modules imported here) with their own host/company-specific
# modules in their own `systemConfigs.<host>` — see the downstream template's
# `system-manager/config/workhost.nix` for that pattern.
{inputs, ...}: {
  specialArgs = {inherit inputs;};

  modules = [
    inputs.self.systemModules.core

    {
      # system-manager builds its own `pkgs` from this `hostPlatform`, so setting
      # it is what wires the whole configuration to x86_64-linux.
      nixpkgs.hostPlatform = "x86_64-linux";

      # Enable the Nix daemon. `nix.enable` defaults to `false` in
      # system-manager; with it off, activating a generation *removes* the
      # nix-daemon units system-manager previously managed, tearing down the
      # daemon. Keep it on for any host that relies on Nix.
      nix.enable = true;

      # system-manager officially supports NixOS, Ubuntu and Debian; opt in to
      # close derivatives too.
      system-manager.allowAnyDistro = true;
    }
  ];
}
