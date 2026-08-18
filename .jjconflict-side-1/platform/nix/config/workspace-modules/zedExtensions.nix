{
  lib,
  config,
  ...
}: let
  inherit (lib) mkOption mkIf;
  inherit (lib.types) lazyAttrsOf raw;
in {
  options.zedExtensions = mkOption {
    type = lazyAttrsOf raw;
    default = {};
    description = "Zed editor extension sources";
  };

  config = {
    outputs = mkIf (config.zedExtensions != {}) {inherit (config) zedExtensions;};
  };
}
