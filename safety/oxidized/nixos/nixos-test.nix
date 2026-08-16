# NixOS VM integration test for oxidized-nixos.
#
# Boots a NixOS VM using oxidized-systemd as PID 1 and verifies that the system
# reaches multi-user.target with core services running.
#
# Run with: nix build .#checks.x86_64-linux.oxidized-nixos-boot
{pkgs}: let
  rustSystemdPackage = pkgs.oxidized-systemd-systemd.override {
    inherit (pkgs) oxidized-systemd;
  };
in
  pkgs.testers.nixosTest {
    name = "oxidized-nixos-boot";

    nodes.machine = {
      config,
      lib,
      pkgs,
      ...
    }: let
      udevRulesOverride = pkgs.runCommand "oxidized-systemd-udev-rules-override" {} ''
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

      # Use oxidized-systemd as the systemd package
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
          settings.Resolve = {
            DNSSEC = "allow-downgrade";
            LLMNR = "true";
            FallbackDNS = ["1.1.1.1" "8.8.8.8"];
          };
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
