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
  inherit (lib) filter mapAttrs mapAttrsToList removeAttrs tryEval;

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

  # Folding in the union of every shell means the default shell is only as
  # portable as the least portable one: devShells.mojo-{gui,wasm} name pkgs.mojo,
  # which has no aarch64-linux build, so on armitas plain `nix develop` was a
  # checkMeta throw rather than a shell. Drop what this system cannot build and
  # keep the rest.
  #
  # tryEval takes any evaluation failure, not just an unsupported platform, so a
  # shell broken by a mistake leaves the union quietly. Asking for it by name -
  # `nix develop .#mojo-gui` - still fails, with the error this discards.
  buildable = shell: (tryEval shell.drvPath).success;
in {
  config.devShells.default = pkgs: let
    otherShells =
      filter buildable
      (mapAttrsToList
        (_: shellFn: resolveDevShell pkgs shellFn)
        (removeAttrs config.devShells ["default"]));
  in
    mkDevShell pkgs [
      ../devshell/modules/common.nix
      ../devshell/modules/git-hooks.nix
      ../devshell/modules/configs/default.nix
      {config.inputsFrom = otherShells;}
    ];
}
