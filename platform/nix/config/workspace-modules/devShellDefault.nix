# Builds the default devShell from pure devshell modules (platform/nix/config/devshell), and
# folds every other named devShell into it via inputsFrom so `nix develop`
# gives the union of all project shells plus the shared tooling and git-hooks.
#
# Replaces the old devenv-based devenvConfigurations module.
{
  config,
  lib,
  inputs,
  ...
}: let
  inherit (lib) mapAttrs mapAttrsToList removeAttrs;

  mkDevShell = import ../devshell/lib/mkDevShell.nix {inherit lib inputs;};

  # Resolve a flakelight devShell value (pkgs: cfg) into a derivation, so it can
  # be pulled into the default shell's inputsFrom. Mirrors flakelight's
  # genDevShell: use overrideShell directly when set, otherwise unwrap the
  # optFunctionTo values by calling them with pkgs. hardeningDisable/overrideShell
  # are never forced here (laziness), so their non-functor values are fine.
  resolveDevShell = pkgs: shellFn: let
    cfg = shellFn pkgs;
  in
    if cfg ? overrideShell && cfg.overrideShell != null
    then cfg.overrideShell
    else let
      cfg' = mapAttrs (_: v: v pkgs) cfg;
    in
      pkgs.mkShell.override {inherit (cfg') stdenv;}
      (cfg'.env
        // {
          inherit (cfg') inputsFrom packages shellHook;
          inherit (cfg) hardeningDisable;
        });
in {
  config.devShells.default = pkgs: let
    otherShells =
      mapAttrsToList
      (_: shellFn: resolveDevShell pkgs shellFn)
      (removeAttrs config.devShells ["default"]);
  in
    mkDevShell pkgs [
      ../devshell/modules/common.nix
      ../devshell/modules/git-hooks.nix
      ../devshell/modules/configs/default.nix
      {config.inputsFrom = otherShells;}
    ];
}
