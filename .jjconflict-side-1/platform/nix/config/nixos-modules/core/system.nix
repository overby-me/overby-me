{
  stateVersion,
  src,
  lib,
  ...
}: {
  # Disable logrotate config validation at build time — the check runs
  # `logrotate --debug` which calls `id` to resolve user/group names,
  # but /etc/passwd doesn't exist inside the Nix build sandbox, causing:
  #   "id: cannot find name for user ID 0"
  services.logrotate.checkConfig = false;

  system = {
    inherit stateVersion;
    # Store copy of all Nix files in /nix/var/nix/profiles/system/full-config
    systemBuilderCommands = let
      nixFiles =
        lib.filterSource (
          path: type:
            type == "directory" || lib.match ".*\\.nix$" (baseNameOf path) != null
        )
        src;
    in "ln -s ${nixFiles} $out/full-config";
  };
}
