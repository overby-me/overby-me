# NixOS workspace example
#
# This workspace demonstrates NixOS machine configuration with nix-workspace.
# It defines a machine, NixOS modules, and a development shell for managing
# the configuration.
#
# Usage:
#   nix build .#nixosConfigurations.my-machine.config.system.build.toplevel
#   nix develop
{
  name = "nixos-example",
  description = "Example NixOS machine configuration using nix-workspace",

  systems = ["x86_64-linux", "aarch64-linux"],

  nixpkgs = {
    allow-unfree = true,
  },

  # NixOS machine configurations (also auto-discovered from machines/)
  machines = {
    my-machine = {
      system = "x86_64-linux",
      state-version = "25.05",

      # Reference modules by name — resolved from the modules/ directory
      modules = ["desktop"],

      host-name = "nixos-example",

      networking = {
        firewall = {
          enable = true,
          allowed-tcp-ports = [22, 80, 443],
        },
      },

      time-zone = "Europe/Copenhagen",
      locale = "en_US.UTF-8",

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

      users = {
        alice = {
          extra-groups = ["wheel", "networkmanager", "video"],
          home-manager = false,
        },
      },
    },
  },

  # NixOS modules (also auto-discovered from modules/)
  modules = {
    desktop = {
      description = "Desktop environment with basic GUI applications",
    },
  },

  # Development shell for managing this NixOS configuration
  shells = {
    default = {
      packages = ["nixos-rebuild", "git"],
      shell-hook = m%"
        echo "NixOS configuration workspace"
        echo "  Build:  nix build .#nixosConfigurations.my-machine.config.system.build.toplevel"
        echo "  Switch: sudo nixos-rebuild switch --flake ."
      "%,
    },
  },
}
