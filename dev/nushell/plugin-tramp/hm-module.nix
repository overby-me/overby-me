# Home Manager module for nushell-plugin-tramp
#
# Usage in your Home Manager configuration:
#
#   # flake.nix
#   {
#     inputs.nushell-plugin-tramp.url = "...";
#   }
#
#   # home.nix
#   { pkgs, inputs, ... }:
#   {
#     imports = [ inputs.nushell-plugin-tramp.homeManagerModules.default ];
#
#     programs.nushell-plugin-tramp = {
#       enable = true;
#       # package = inputs.nushell-plugin-tramp.packages.${pkgs.system}.default;
#       # cacheTTL = "5sec";
#     };
#   }
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.nushell-plugin-tramp;
in {
  options.programs.nushell-plugin-tramp = {
    enable = lib.mkEnableOption "nushell-plugin-tramp, a TRAMP-inspired remote filesystem plugin for Nushell";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.nushell-plugin-tramp;
      defaultText = lib.literalExpression "pkgs.nushell-plugin-tramp";
      description = "The nushell-plugin-tramp package to use.";
    };

    cacheTTL = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "10sec";
      description = ''
        Default cache TTL for stat and directory listing caches.
        Set as `$env.TRAMP_CACHE_TTL` in your Nushell environment.
        Accepts integer seconds (`5`), float seconds (`2.5`), or a
        duration string with a suffix (`500ms`, `10s`, `3sec`).
        When null, the built-in default of 5 seconds is used.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    # Ensure the plugin binary is available on PATH.
    home.packages = [cfg.package];

    # Register the plugin with Nushell via the native plugin list.
    programs.nushell = {
      extraConfig = lib.mkAfter (
        lib.concatStringsSep "\n" (
          lib.filter (s: s != "") [
            # Register the plugin binary so Nushell discovers it on startup.
            "plugin add ${lib.getExe cfg.package}"
            # Set the cache TTL environment variable if configured.
            (lib.optionalString (cfg.cacheTTL != null) "$env.TRAMP_CACHE_TTL = '${cfg.cacheTTL}'")
          ]
        )
      );
    };
  };
}
