{
  lib,
  config,
  ...
}: let
  inherit (lib) mkOption mkIf;
  inherit (lib.types) lazyAttrsOf raw;
in {
  options.users = mkOption {
    type = lazyAttrsOf raw;
    default = {};
    description = "User home-manager configuration file paths";
  };

  config = {
    outputs = mkIf (config.users != {}) {inherit (config) users;};
    nixDirPathAttrs = ["users"];
  };
}
