# Automatically names each devShell derivation to "${attr-name}-shell".
#
# Replaces flakelight's default outputs.devShells generation with one that
# passes `name` directly into mkShell during creation, or applies
# overrideAttrs for pre-built shells (e.g. devenv).
{
  config,
  lib,
  genSystems,
  ...
}: let
  inherit (lib) mapAttrs mkForce;

  # Replicate flakelight's genDevShell, injecting the shell name.
  genNamedDevShell = name: pkgs: cfg:
    if cfg.overrideShell != null
    then cfg.overrideShell.overrideAttrs {name = "${name}-shell";}
    else let
      cfg' = mapAttrs (_: v: v pkgs) cfg;
    in
      pkgs.mkShell.override {inherit (cfg') stdenv;}
      (cfg'.env
        // {
          name = "${name}-shell";
          inherit (cfg') inputsFrom packages shellHook;
          inherit (cfg) hardeningDisable;
        });
in {
  config.outputs.devShells = mkForce (genSystems (pkgs:
    mapAttrs (name: v: genNamedDevShell name pkgs (v pkgs))
    config.devShells));
}
