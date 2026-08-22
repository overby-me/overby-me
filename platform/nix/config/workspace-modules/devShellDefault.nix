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

  # Resolve a devShell value (pkgs: cfg) into a derivation, so it can be pulled
  # into the default shell's inputsFrom. Mirrors the framework's own
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
  # portable as the least portable one: one shell naming a package this system
  # cannot build turns plain `nix develop` into a checkMeta throw rather than a
  # shell. That is what devShells.mojo-{gui,wasm} did on armitas until mojo
  # started building from source on aarch64-linux, and what the next
  # single-platform package to reach a shell would do again. Drop what this
  # system cannot build and keep the rest.
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
