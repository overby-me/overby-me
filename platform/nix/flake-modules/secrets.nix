{
  lib,
  config,
  ...
}: let
  inherit (lib) mkOption mkIf filterAttrs hasSuffix mapAttrs' removeSuffix;
  inherit (lib.types) lazyAttrsOf raw;

  # Named directly rather than through nixDir: a secret is not a module to
  # import, it is an .age file to point at, so this scans by extension and
  # has no business following the output-directory convention.
  dir = ../secrets;
  ageFiles =
    if lib.pathExists dir
    then
      mapAttrs' (file: _: {
        name = removeSuffix ".age" file;
        value = dir + "/${file}";
      }) (filterAttrs (file: _: hasSuffix ".age" file) (lib.readDir dir))
    else {};
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
