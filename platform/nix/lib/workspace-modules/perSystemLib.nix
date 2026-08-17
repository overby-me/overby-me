# Per-system lib functions.
#
# Modules can define per-system lib attrs via the `perSystemLib` option.
# Each value is a function taking pkgs and returning a value.
# These are injected into pkgs.lib via an overlay, making them available
# in callPackage-style package definitions as `lib.<name>`.
#
# Example:
#   {
#     perSystemLib.buildDenoProject = pkgs:
#       import ./buildDenoProject.nix { inherit (pkgs) lib stdenvNoCC deno fetchurl jq writeText; };
#   }
#
# Then in a package definition:
#   packages.my-app = { lib, ... }:
#     lib.buildDenoProject { ... };
{
  config,
  lib,
  ...
}: {
  options.perSystemLib = lib.mkOption {
    type = with lib.types; attrsOf (functionTo anything);
    default = {};
    description = "Per-system lib functions. Each value is a function from pkgs to a value, injected into pkgs.lib.";
  };

  config = lib.mkIf (config.perSystemLib != {}) {
    withOverlays = [
      (final: prev: {
        lib = prev.lib.extend (
          _: _:
            lib.mapAttrs (_: fn: fn final) config.perSystemLib
        );
      })
    ];
  };
}
