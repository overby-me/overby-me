{
  pkgs,
  inputs,
  ...
}: let
  inherit (inputs.self.secrets) publicKeys;
in {
  environment.profiles = ["$HOME/.local"];
  users.users."overby.me" = {
    shell = pkgs.pkgsUnstable.nushell;
    isNormalUser = true;
    description = "Niclas Overby";
    extraGroups = ["networkmanager" "wheel" "docker" "libvirtd" "wireshark" "input" "kvm"];
    openssh.authorizedKeys.keys = [publicKeys.overby-me-ssh-ed25519];
  };
}
