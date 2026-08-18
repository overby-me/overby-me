{
  pkgs,
  inputs,
  lib,
  stateVersion,
  src,
  ...
}: {
  # home-manager takes homeDirectory from users.users.<name>.home, which is
  # null until the account is declared, so it has to be declared before
  # home-manager.users below can evaluate at all. Setting only the path leaves
  # macOS account management alone: nix-darwin creates and modifies real
  # accounts only for names listed in users.knownUsers.
  users.users = lib.mapAttrs (name: _: {home = "/Users/${name}";}) inputs.self.users;

  home-manager = {
    # Same set as the NixOS side. Without it home-manager is configured but has
    # nobody to configure, so everything under home-manager/modules (the shell,
    # the editors, the agent skills in claude-code) silently stops at the
    # Linux hosts.
    inherit (inputs.self) users;
    useGlobalPkgs = true;
    useUserPackages = true;
    backupFileExtension = "hm-backup";
    extraSpecialArgs = {
      inherit
        inputs
        pkgs
        stateVersion
        src
        ;
    };
  };
}
