{
  config,
  lib,
  flakelight,
  moduleArgs,
  ...
}: let
  inherit (lib) mkOption mkIf mkMerge;
  inherit (lib.types) lazyAttrsOf;
  inherit (flakelight.types) module nullable optCallWith;
in {
  options = {
    devenvModule = mkOption {
      type = nullable module;
      default = null;
      description = "Default devenv module to export";
    };

    devenvModules = mkOption {
      type = optCallWith moduleArgs (lazyAttrsOf module);
      default = {};
      description = "Devenv modules to export";
    };
  };

  config = mkMerge [
    (mkIf (config.devenvModule != null) {
      devenvModules.default = config.devenvModule;
    })

    (mkIf (config.devenvModules != {}) {
      outputs = {inherit (config) devenvModules;};
    })

    {nixDirPathAttrs = ["devenvModules"];}
  ];
}
