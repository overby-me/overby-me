# Run a single upstream systemd integration test against rust-systemd.
#
# Boots a NixOS VM with rust-systemd as PID 1, installs the upstream test
# scripts and test data, then runs the specified test and checks for /testok.
#
# Run with: nix build .#checks.x86_64-linux.rust-systemd-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-systemd-test-01-BASIC
{
  pkgs,
  name,
}: let
  systemdSrc = pkgs.systemd.src;

  # Build a derivation containing all test scripts and testdata from upstream
  testScripts = pkgs.runCommand "systemd-test-scripts" {} ''
    mkdir -p $out/{units,testdata}

    # Copy test runner scripts (test/units/)
    cp -a ${systemdSrc}/test/units/* $out/units/

    # Copy integration test data (unit files, helper scripts, etc.)
    cp -a ${systemdSrc}/test/testdata/integration-tests/* $out/testdata/
  '';

  testName = "TEST-${name}";

  rustSystemdPackage = pkgs.rust-systemd-systemd.override {
    rust-systemd = pkgs.rust-systemd-drowse;
  };
in
  pkgs.testers.nixosTest {
    name = "rust-systemd-test-${name}";

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

      environment = {
        # Install test scripts and testdata into the VM
        etc."systemd-tests/units".source = "${testScripts}/units";
        etc."systemd-tests/testdata".source = "${testScripts}/testdata";

        # Packages needed by test scripts
        systemPackages = with pkgs; [
          bash
          coreutils
          diffutils
          gnugrep
          gnused
          gawk
          findutils
          iproute2
          util-linux
          jq
          procps
          kmod
        ];
      };

      # Install test-specific unit files into systemd unit path if they exist
      system.activationScripts.systemd-test-units = ''
        UNITS_DIR="/etc/systemd-tests/testdata/${testName}/${testName}.units"
        if [ -d "$UNITS_DIR" ]; then
          mkdir -p /run/systemd/system
          for f in "$UNITS_DIR"/*; do
            name=$(basename "$f")
            case "$name" in
              *.service|*.socket|*.target|*.timer|*.path|*.mount|*.slice|*.scope|*.swap)
                cp -a "$f" "/run/systemd/system/$name"
                ;;
              *.sh)
                # Copy helper scripts preserving executability
                cp -a "$f" "/run/systemd/system/$name"
                chmod +x "/run/systemd/system/$name"
                ;;
              *.wants|*.requires)
                cp -a "$f" "/run/systemd/system/$name"
                ;;
            esac
          done
        fi
      '';

      users.users.nixos = {
        isNormalUser = true;
        extraGroups = ["wheel"];
        password = "nixos";
      };

      # Give the VM enough resources for tests
      virtualisation = {
        memorySize = 1024;
        cores = 2;
      };
    };

    testScript = ''
      machine.wait_for_unit("multi-user.target", timeout=120)

      # Reload systemd to pick up any test unit files installed via activation
      machine.succeed("systemctl daemon-reload")

      # Run the upstream test script.
      # Tests source util.sh and test-control.sh from $(dirname "$0"),
      # so we run from the units directory.
      (rc, output) = machine.execute(
          "cd /etc/systemd-tests/units && "
          "bash -x ./${testName}.sh 2>&1"
      )

      print(output)

      if rc != 0:
          raise Exception("${testName} failed with exit code " + str(rc))

      # Check for /testok (standard systemd test success marker)
      machine.succeed("test -f /testok")
    '';
  }
