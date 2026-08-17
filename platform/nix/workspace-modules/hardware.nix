{
  lib,
  config,
  ...
}: let
  inherit (lib) mkOption mkIf;
  inherit (lib.types) lazyAttrsOf raw;
in {
  options.hardware = mkOption {
    type = lazyAttrsOf raw;
    default = {};
    description = "Hardware configuration NixOS modules";
  };

  config = {
    outputs = mkIf (config.hardware != {}) {inherit (config) hardware;};
    nixDirPathAttrs = ["hardware"];
  };
}
