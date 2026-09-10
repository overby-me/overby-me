{
  pkgs,
  inputs,
  ...
}: let
  inherit (inputs.self.secrets) publicKeys;
  nushell = pkgs.pkgsUnstable.nushell;
in {
  environment.profiles = ["$HOME/.local"];

  # pkexec matches $SHELL against /etc/shells literally, exiting 127 on a miss.
  # Both spellings: passwd holds the profile path, the home module the store one.
  environment.shells = [nushell "${nushell}/bin/nu"];

  users.users."overby.me" = {
    shell = nushell;
    isNormalUser = true;
    description = "Niclas Overby";
    extraGroups = ["networkmanager" "wheel" "docker" "libvirtd" "wireshark" "input" "kvm"];
    openssh.authorizedKeys.keys = [publicKeys.overby-me-ssh-ed25519];
  };
}
