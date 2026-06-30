{
  lib,
  config,
  ...
}: let
  inherit (lib) mkOption mkIf filterAttrs hasSuffix mapAttrs' removeSuffix mergeAttrsList;
  inherit (lib.types) lazyAttrsOf raw;

  aliasNames = ["secrets"] ++ (config.nixDirAliases.secrets or []);
  ageFiles = mergeAttrsList (map (
      name: let
        dir = config.nixDir + "/${name}";
      in
        if lib.pathExists dir
        then
          mapAttrs' (file: _: {
            name = removeSuffix ".age" file;
            value = dir + "/${file}";
          }) (filterAttrs (file: _: hasSuffix ".age" file) (lib.readDir dir))
        else {}
    )
    aliasNames);
in {
  options.secrets = mkOption {
    type = lazyAttrsOf raw;
    default = {};
    description = "Age-encrypted secret file paths and public key metadata";
  };

  config = {
    secrets = ageFiles;
    outputs = mkIf (config.secrets != {}) {inherit (config) secrets;};
  };
}
