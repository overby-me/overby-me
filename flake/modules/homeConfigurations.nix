# Flakelight module replacing the built-in `homeConfigurations` output.
#
# Flakelight's stock `homeConfigurations` builtin builds each standalone
# home-manager configuration with a bare, un-overlaid package set:
#
#     pkgs = inputs.nixpkgs.legacyPackages.${cfg.system};
#
# and it does *not* expose `pkgs` as a specialArg. That leaks two problems into
# every user config:
#
#   1. Infinite recursion. Modules that branch their `imports` on the target
#      platform (e.g. `lib.optionals pkgs.stdenv.isDarwin [...]`) reference the
#      `pkgs` module argument, which — with no specialArg to satisfy it — is
#      resolved from `_module.args` (i.e. `config.nixpkgs`). Reading `config`
#      while computing `imports` forms a `config -> imports -> pkgs -> config`
#      cycle. (The nix-darwin entry point avoids this only because it happens to
#      pass `pkgs` as a specialArg.)
#
#   2. Missing overlays. Even though flakelight's `propagationModule` forwards
#      the flake overlays into `config.nixpkgs.overlays` (so home-manager's own
#      `pkgs` is overlaid), the bare `pkgs` the builtin passes is not — so any
#      value coming from an overlay (`rust-bin`, `pkgsUnstable`, ...) is absent
#      if a user config ever reaches for the "builtin" pkgs.
#
# This module disables the stock builtin (via `disabledModules`) and rebuilds
# each home configuration with flakelight's fully-overlaid, patched
# `pkgsFor.${system}`, exposing that same package set as the `pkgs` specialArg.
# User configs can then use `pkgs.stdenv.is*` in `imports` freely, with no
# recursion and with all flake overlays available.
#
# It mirrors the repo's own darwin.nix pattern of owning a flakelight-style
# configuration output. The stock builtin's `outputs` type uses a custom
# priority-ignoring merge, so overriding its output with `mkForce` is not
# possible — the builtin must instead be disabled outright.
{
  config,
  lib,
  inputs,
  flakelight,
  pkgsFor,
  outputs,
  moduleArgs,
  modulesPath,
  ...
}: let
  inherit (builtins) head mapAttrs match;
  inherit (lib) foldl mapAttrsToList mkIf mkOption recursiveUpdate;
  inherit (lib.types) attrs lazyAttrsOf;
  inherit (flakelight) selectAttr;
  inherit (flakelight.types) optCallWith;

  isHome = x: x ? activationPackage;

  mkHome = name: cfg: let
    inherit (cfg) system;
    # Fully-overlaid, patched package set — identical to what the flake uses
    # everywhere else and to what home-manager's own `config.nixpkgs` resolves
    # to via the propagationModule.
    pkgs = pkgsFor.${system};
  in
    inputs.home-manager.lib.homeManagerConfiguration (
      (removeAttrs cfg ["system"])
      // {
        inherit pkgs;
        extraSpecialArgs =
          {
            inherit inputs pkgs;
            inputs' = mapAttrs (_: selectAttr system) inputs;
            outputs' = selectAttr system outputs;
          }
          // cfg.extraSpecialArgs or {};
        modules =
          [
            ({lib, ...}: {
              home.username = lib.mkDefault (head (match "([^@]*)(@.*)?" name));
            })
            config.propagationModule
          ]
          ++ cfg.modules or [];
      }
    );

  configs =
    mapAttrs
    (name: cfg:
      if isHome cfg
      then cfg
      else mkHome name cfg)
    config.homeConfigurations;
in {
  # The stock builtin builds home configs with a bare, un-overlaid pkgs and no
  # `pkgs` specialArg; disable it and take over the option/outputs below.
  disabledModules = [(modulesPath + "/homeConfigurations.nix")];

  options.homeConfigurations = mkOption {
    type = optCallWith moduleArgs (lazyAttrsOf (optCallWith moduleArgs attrs));
    default = {};
  };

  config = {
    outputs = mkIf (config.homeConfigurations != {}) {
      homeConfigurations = configs;
      checks = foldl recursiveUpdate {} (
        mapAttrsToList
        (n: v: {
          ${v.config.nixpkgs.system}."home-${n}" = v.activationPackage;
        })
        configs
      );
    };
    nixDirAliases.homeConfigurations = ["home"];
  };
}
