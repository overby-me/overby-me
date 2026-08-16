({
  modulesPath,
  lib,
  ...
}: {
  imports = ["${modulesPath}/profiles/qemu-guest.nix"];

  system = {
    name = "rust-nixos";
    stateVersion = "25.11";
  };

  boot = {
    kernelParams = ["init=/nix/var/nix/profiles/system/init"];
    loader.grub.enable = false;
  };

  fileSystems."/" = {
    device = "/dev/disk/by-label/nixos";
    autoResize = true;
    fsType = "ext4";
  };

  # Enable systemd-networkd for network management (tests the Rust networkd)
  networking = {
    useNetworkd = true;
    useDHCP = false;
    # Let networkd handle DHCP per-interface via its .network files
  };

  systemd = {
    # Configure networkd to DHCP on all ethernet interfaces
    network = {
      enable = true;
      networks."10-ethernet" = {
        matchConfig.Name = "en* eth*";
        networkConfig = {
          DHCP = "ipv4";
          IPv6AcceptRA = true;
        };
        dhcpV4Config = {
          UseDNS = true;
          UseRoutes = true;
        };
      };
    };

    services = {
      # Workaround: disable PrivateDevices for resolved and timesyncd.
      # The Rust exec_helper's mount namespace setup does not yet fully support
      # PrivateDevices=yes — services crash with SIGABRT inside the namespace.
      # The upstream unit files set PrivateDevices=yes, so we override it here
      # until the exec_helper implementation is fixed.
      systemd-resolved.serviceConfig.PrivateDevices = lib.mkForce false;
      systemd-timesyncd.serviceConfig.PrivateDevices = lib.mkForce false;

      # Disable systemd-networkd-wait-online.service — the upstream C binary tries
      # to talk to networkd via varlink/D-Bus, but the Rust networkd doesn't
      # implement those interfaces yet.  The service currently exits quickly
      # (connection refused), so it doesn't block boot, but keeping it disabled
      # avoids a spurious failure in the activation graph.
      systemd-networkd-wait-online.enable = lib.mkForce false;

      # Disable lvm-devices-import.service — this VM does not use LVM and the
      # lvm binary is not included in the system closure, so the service fails
      # with CannotFindBinaryPath at every boot.
      lvm-devices-import.enable = lib.mkForce false;
    };
  };

  services = {
    # Disable logrotate config validation at build time — the check runs
    # `logrotate --debug` which calls `id` to resolve user/group names,
    # but /etc/passwd doesn't exist inside the Nix build sandbox, causing:
    #   "id: cannot find name for user ID 0"
    logrotate.checkConfig = false;

    # Enable systemd-resolved for DNS resolution (tests the Rust resolved)
    resolved = {
      enable = true;
      settings.Resolve = {
        DNSSEC = "allow-downgrade";
        LLMNR = "true";
        FallbackDNS = ["1.1.1.1" "8.8.8.8"];
      };
    };

    getty.autologinUser = "nixos";
  };

  users.users = {
    nixos = {
      isNormalUser = true;
      extraGroups = ["wheel"];
      password = "nixos";
    };
  };
})
