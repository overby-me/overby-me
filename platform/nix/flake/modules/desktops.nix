{
  lib,
  config,
  ...
}: let
  inherit (lib) mkOption mkIf;
  inherit (lib.types) lazyAttrsOf raw;
in {
  options.desktops = mkOption {
    type = lazyAttrsOf raw;
    default = {};
    description = "Combined NixOS + home-manager desktop configuration modules";
  };

  config = {
    outputs = mkIf (config.desktops != {}) {inherit (config) desktops;};
    nixDirPathAttrs = ["desktops"];
  };
}
