{
  lib,
  hasSecrets ? true,
  ...
}: {
  options.hasSecrets = lib.mkOption {
    type = lib.types.bool;
    default = hasSecrets;
    description = "Whether this host has access to secrets.";
  };
}
