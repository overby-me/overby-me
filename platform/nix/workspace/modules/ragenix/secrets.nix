{
  lib,
  config,
  ...
}: let
  inherit (lib) mkOption mkIf filterAttrs hasSuffix mapAttrs' removeSuffix;
  inherit (lib.types) lazyAttrsOf raw;
  inherit (lib) types;

  # Named directly rather than through nixDir: a secret is not a module to
  # import, it is an .age file to point at, so this scans by extension and
  # has no business following the output-directory convention.
  # Where the .age files are is the consuming tree's business, not this
  # integration's: reaching back into it with a relative path is how this
  # module came to find nothing when it moved. A secret is not a module to
  # import either - it is a file to point at - so this scans by extension
  # rather than following the output-directory convention.
  dir = config.secretsDir;
  ageFiles =
    if dir != null && lib.pathExists dir
    then
      mapAttrs' (file: _: {
        name = removeSuffix ".age" file;
        value = dir + "/${file}";
      }) (filterAttrs (file: _: hasSuffix ".age" file) (lib.readDir dir))
    else {};
in {
  options.secretsDir = mkOption {
    type = types.nullOr types.path;
    default = null;
    description = "Directory of .age files, in the tree that owns them.";
  };

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
