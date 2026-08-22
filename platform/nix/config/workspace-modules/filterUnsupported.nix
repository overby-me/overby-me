# Drop packages that are unsupported on the current system.
#
# `nix flake check` (and `nix flake show`) forces every `packages.<system>.*`
# derivation. nixpkgs' checkMeta throws ("Refusing to evaluate ... not available
# on the requested hostPlatform") when a derivation's meta.platforms excludes the
# evaluating system, which aborts evaluation on e.g. aarch64-darwin even though
# those packages are Linux-only by design.
#
# This module regenerates the `packages` output per system from the framework's
# own packageOverlay, keeping only entries whose `meta.available` is true for
# that system. It mirrors the packages.nix generation rather than reading
# `config.outputs.packages` (which would recurse against this mkForce override).
{
  lib,
  config,
  genSystems,
  ...
}: let
  inherit
    (lib)
    attrNames
    filterAttrs
    genAttrs
    mkForce
    tryEval
    ;

  # A derivation is "supported" when forcing meta.available succeeds and is
  # true. tryEval guards against packages whose meta itself fails to evaluate.
  isSupported = drv: let
    r = tryEval (drv.meta.available or true);
  in
    r.success && r.value;

  # Per system: { name = pkgs.name; } for every package added via the
  # packageOverlay, excluding the synthetic "default" alias and unsupported ones.
  supportedPackages = genSystems (
    pkgs: let
      names = lib.subtractLists ["default"] (attrNames (config.packageOverlay pkgs pkgs));
    in
      filterAttrs (_: isSupported) (genAttrs names (n: pkgs.${n}))
  );
in {
  config.outputs.packages = mkForce supportedPackages;
}
