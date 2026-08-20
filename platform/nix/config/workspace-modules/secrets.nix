# The `secrets` output the ragenix integration used to declare, kept so
# outputs.nix goes on feeding it from secrets/: today that is publicKeys.nix,
# which the hosts read for authorized_keys and image recipients.
{
  config,
  lib,
  ...
}: {
  options.secrets = lib.mkOption {
    type = lib.types.lazyAttrsOf lib.types.raw;
    default = {};
    description = "Entries of secrets/, by name.";
  };

  config.outputs = lib.mkIf (config.secrets != {}) {inherit (config) secrets;};
}
