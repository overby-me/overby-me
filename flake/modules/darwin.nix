# Flakelight module adding nix-darwin support.
#
# Flakelight has no built-in darwinConfigurations/darwinModules outputs, so this
# mirrors flakelight's own nixosConfigurations.nix / nixosModules.nix builtins:
#
#   - `darwinModules`        — discovered from darwin/modules, exported as the
#                              flake's `darwinModules` output.
#   - `darwinConfigurations` — discovered from darwin/config, each entry built
#                              with `nix-darwin.lib.darwinSystem` and exported as
#                              the flake's `darwinConfigurations` output.
#
# As with the NixOS builtins, the flakelight `propagationModule` is injected so
# overlays and nixpkgs.config (and, when home-manager's nix-darwin module is
# used, home-manager.sharedModules) are forwarded into each configuration.
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
  inherit
    (lib)
    mapAttrsToList
    mkIf
    mkMerge
    mkOption
    ;
  inherit (lib.types) attrs lazyAttrsOf;
  inherit (flakelight.types) module optCallWith;

  # Avoid checking if toplevel is a derivation as it causes the modules to be
  # evaluated (matches flakelight's nixosConfigurations isNixos check).
  isDarwin = x: x ? config.system.build.toplevel;

  mkDarwin = hostname: cfg:
    inputs.nix-darwin.lib.darwinSystem (
      cfg
      // {
        specialArgs =
          {
            inherit inputs hostname;
          }
          // cfg.specialArgs or {};
        modules =
          [
            config.propagationModule
            (
              {flake, ...}: {
                _module.args = {inherit (flake) inputs';};
              }
            )
          ]
          ++ cfg.modules or [];
      }
    );

  configs =
    mapAttrs (
      hostname: cfg:
        if isDarwin cfg
        then cfg
        else mkDarwin hostname cfg
    )
    config.darwinConfigurations;
in {
  options = {
    darwinModules = mkOption {
      type = optCallWith moduleArgs (lazyAttrsOf module);
      default = {};
    };

    darwinConfigurations = mkOption {
      type = optCallWith moduleArgs (lazyAttrsOf (optCallWith moduleArgs attrs));
      default = {};
    };
  };

  config = mkMerge [
    (mkIf (config.darwinModules != {}) {
      outputs = {inherit (config) darwinModules;};
    })

    (mkIf (config.darwinConfigurations != {}) {
      outputs.darwinConfigurations = configs;

      # Expose each system's toplevel as a check, wrapped in a runCommand so
      # computing its name stays cheap (matches the NixOS builtin).
      outputs.checks = mkMerge (
        mapAttrsToList (
          n: v: let
            inherit (v.pkgs.stdenv.buildPlatform) system;
          in {
            ${system}."darwin-${n}" =
              pkgsFor.${system}.runCommand "check-darwin-${n}" {}
              "echo ${v.config.system.build.toplevel} > $out";
          }
        )
        configs
      );
    })

    {nixDirPathAttrs = ["darwinModules"];}
  ];
}
