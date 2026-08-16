# Example NixOS machine configuration (auto-discovered from machines/)
#
# This file is auto-discovered by nix-workspace from the machines/ directory.
# It produces a nixosConfigurations.my-machine flake output.
#
# When both this file and an inline declaration in workspace.ncl exist for
# the same machine name, they are merged (with workspace.ncl taking precedence).
#
# This demonstrates the convention-based approach: drop a .ncl file in
# machines/ and nix-workspace picks it up automatically.
{
  system = "x86_64-linux",
  state-version = "25.05",

  host-name = "nixos-example",

  modules = ["desktop"],

  boot-loader = 'systemd-boot,

  file-systems = {
    "/" = {
      device = "/dev/disk/by-label/nixos",
      fs-type = "ext4",
    },
    "/boot" = {
      device = "/dev/disk/by-label/boot",
      fs-type = "vfat",
      needed-for-boot = true,
    },
  },

  networking = {
    firewall = {
      enable = true,
      allowed-tcp-ports = [22, 80, 443],
    },
  },

  time-zone = "Europe/Copenhagen",
  locale = "en_US.UTF-8",

  users = {
    alice = {
      extra-groups = ["wheel", "networkmanager", "video"],
      home-manager = false,
    },
  },
}
