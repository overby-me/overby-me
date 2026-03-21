# NixOS VM integration test for rust-nixos.
#
# Boots a NixOS VM using rust-systemd as PID 1 and verifies that the system
# reaches multi-user.target with core services running.
#
# Run with: nix build .#checks.x86_64-linux.rust-nixos-boot
{pkgs}: let
  rustSystemdPackage = pkgs.rust-systemd-systemd.override {
    rust-systemd = pkgs.rust-systemd-drowse;
  };
in
  pkgs.testers.nixosTest {
    name = "rust-nixos-boot";

    nodes.machine = {
      config,
      lib,
      pkgs,
      ...
    }: let
      udevRulesOverride = pkgs.runCommand "rust-systemd-udev-rules-override" {} ''
        mkdir -p $out/lib/udev/rules.d
        for rule in ${config.systemd.package}/lib/udev/rules.d/*.rules; do
          if grep -q 'systemctl' "$rule"; then
            cp "$rule" "$out/lib/udev/rules.d/$(basename "$rule")"
          fi
        done
      '';
    in {
      imports = [./bash.nix];

      system.stateVersion = "25.11";

      # Use rust-systemd as the systemd package
      systemd.package = rustSystemdPackage;
      services.udev.packages = [udevRulesOverride];

      # sudo-rs
      security.sudo.enable = false;
      security.sudo-rs = {
        enable = true;
        wheelNeedsPassword = false;
      };

      # Network configuration (tests rust networkd)
      networking = {
        useNetworkd = true;
        useDHCP = false;
      };

      systemd = {
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
          systemd-resolved.serviceConfig.PrivateDevices = lib.mkForce false;
          systemd-timesyncd.serviceConfig.PrivateDevices = lib.mkForce false;
          systemd-networkd-wait-online.enable = lib.mkForce false;
          lvm-devices-import.enable = lib.mkForce false;
        };
      };

      services = {
        logrotate.checkConfig = false;
        resolved = {
          enable = true;
          dnssec = "allow-downgrade";
          llmnr = "true";
          fallbackDns = ["1.1.1.1" "8.8.8.8"];
        };
      };

      users.users.nixos = {
        isNormalUser = true;
        extraGroups = ["wheel"];
        password = "nixos";
      };
    };

    testScript = ''
      machine.wait_for_unit("multi-user.target", timeout=120)

      # Test journald
      machine.wait_for_unit("systemd-journald.service")

      # Test resolved
      machine.wait_for_unit("systemd-resolved.service")

      # Test networkd
      machine.wait_for_unit("systemd-networkd.service")
    '';
  }
