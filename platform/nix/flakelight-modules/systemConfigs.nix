# A flakelight module that teaches flakelight about `systemConfigs` and
# `systemModules` outputs, for numtide's system-manager
# (https://github.com/numtide/system-manager).
#
# flakelight ships builtin modules for `nixosConfigurations`/`nixosModules`,
# `darwinModules`, `homeConfigurations`, etc., but has no native system-manager
# support. This mirrors those builtins so system-manager configs and their
# reusable modules are first-class, autoloaded outputs rather than hand-written
# `outputs.*` entries.
#
# With this module:
#
#   - `systemModules.<name>` — discovered from `system-manager/modules`, exported
#     as the flake's `systemModules` output (the counterpart to `nixosModules`),
#     so downstream flakes can compose them.
#   - `systemConfigs.<name>` can be set to either
#       * a set of `system-manager.lib.makeSystemConfig` args (an attrset with a
#         `modules` list), which this module then builds, or
#       * an already-built config (the result of calling `makeSystemConfig`),
#         which is passed through untouched.
#   - files under `system-manager/config/` are autoloaded as `systemConfigs`
#     entries (via `nixDirAliases`), mirroring how `darwin/config` and
#     `home-manager/config` autoload — so dropping in a new file is all it takes.
#   - each config gets a `system-<name>` build check.
#
# `inputs.system-manager` must be present.
{
  config,
  lib,
  inputs,
  flakelight,
  moduleArgs,
  pkgsFor,
  ...
}: let
  inherit (builtins) mapAttrs;
  inherit (lib) mapAttrsToList mkIf mkMerge mkOption;
  inherit (lib.types) attrs lazyAttrsOf;
  inherit (flakelight.types) module optCallWith;

  # A built system-manager config is the `pkgs.linkFarm "system-manager" …`
  # derivation, which carries the evaluated module set in `passthru.config`.
  # Checking for that attr (rather than `lib.isDerivation`) avoids forcing the
  # derivation's name, which keeps `nix flake show` fast.
  isBuilt = x: x ? config.build.toplevel;

  mkConfig = _name: cfg:
    if isBuilt cfg
    then cfg
    else inputs.system-manager.lib.makeSystemConfig cfg;

  configs = mapAttrs mkConfig config.systemConfigs;
in {
  options = {
    systemModules = mkOption {
      type = optCallWith moduleArgs (lazyAttrsOf module);
      default = {};
    };

    systemConfigs = mkOption {
      type = optCallWith moduleArgs (lazyAttrsOf (optCallWith moduleArgs attrs));
      default = {};
    };
  };

  config = mkMerge [
    (mkIf (config.systemModules != {}) {
      outputs = {inherit (config) systemModules;};
    })

    (mkIf (config.systemConfigs != {}) {
      outputs.systemConfigs = configs;

      outputs.checks = mkMerge (mapAttrsToList
        (name: cfg: let
          system = cfg.config.nixpkgs.hostPlatform;
        in {
          # Wrap the toplevel in a trivial derivation so `nix flake show`
          # doesn't pay to compute the linkFarm's name, matching how the
          # builtin nixosConfigurations check is written.
          ${system}."system-${name}" =
            pkgsFor.${system}.runCommand "check-system-${name}" {}
            "echo ${cfg} > $out";
        })
        configs);
    })

    {
      nixDirPathAttrs = ["systemModules"];
    }
  ];
}
