{
  pkgs,
  inputs,
  ...
}: let
  inherit (inputs.self.secrets) publicKeys;
in {
  environment.profiles = ["$HOME/.local"];

  # Must list every login shell below: pkexec rejects a caller whose $SHELL is
  # absent from /etc/shells, failing privileged desktop helpers with exit 127.
  environment.shells = [pkgs.pkgsUnstable.nushell];

  users.users."overby.me" = {
    shell = pkgs.pkgsUnstable.nushell;
    isNormalUser = true;
    description = "Niclas Overby";
    extraGroups = ["networkmanager" "wheel" "docker" "libvirtd" "wireshark" "input" "kvm"];
    openssh.authorizedKeys.keys = [publicKeys.overby-me-ssh-ed25519];
  };
}
