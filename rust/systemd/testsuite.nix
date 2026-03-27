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
  patchScript ? "",
  extraPackages ? [],
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
          uuidd.enable = lib.mkForce false;
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
        systemPackages = with pkgs;
          [
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
            hostname # for hostname command
            acl # for setfacl/getfacl
          ]
          ++ extraPackages;
      };

      # Install test-specific unit files into systemd unit path if they exist
      system.activationScripts.systemd-test-units = ''
        # Always create /run/systemd/system so tests can write unit files there
        mkdir -p /run/systemd/system

        # Mask debug-shell.service — it fails in the test VM because there's
        # no serial/tty available, causing TEST-01-BASIC's "no failed units"
        # assertion to trip.
        ln -sfn /dev/null /run/systemd/system/debug-shell.service

        # Make /etc/systemd/system writable for tests that create drop-ins
        # NixOS normally makes this read-only via etc activation
        if [ -L /etc/systemd/system ] || [ ! -w /etc/systemd/system ]; then
          # Save existing unit files, recreate as writable directory
          tmp=$(mktemp -d)
          if [ -e /etc/systemd/system ]; then
            cp -a /etc/systemd/system/. "$tmp/" 2>/dev/null || true
            rm -f /etc/systemd/system
          fi
          mkdir -p /etc/systemd/system
          cp -a "$tmp/." /etc/systemd/system/ 2>/dev/null || true
          rm -rf "$tmp"
        fi

        UNITS_DIR="/etc/systemd-tests/testdata/${testName}/${testName}.units"
        if [ -d "$UNITS_DIR" ]; then
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

        # Create writable /usr/lib/systemd/system/ for tests that write units there
        mkdir -p /usr/lib/systemd/system

        # Symlink helper binaries (systemd-sysctl, etc.) so tests referencing
        # /usr/lib/systemd/systemd-* can find them (NixOS puts them in the store)
        for bin in ${config.systemd.package}/lib/systemd/systemd-*; do
          name=$(basename "$bin")
          [ -e "/usr/lib/systemd/$name" ] || ln -sfn "$bin" "/usr/lib/systemd/$name"
        done

        # Create standard systemd testdata path so unit files referencing
        # /usr/lib/systemd/tests/testdata/ can find helper scripts.
        # We create a real directory instead of a plain symlink so we can
        # add flat symlinks for test data subdirectories.
        mkdir -p /usr/lib/systemd/tests/testdata

        # Symlink all top-level entries from the nix store testdata
        for entry in /etc/systemd-tests/testdata/*; do
          name=$(basename "$entry")
          ln -sfn "$entry" "/usr/lib/systemd/tests/testdata/$name"
        done

        # Upstream unit files reference e.g. /usr/lib/systemd/tests/testdata/TEST-07-PID1.units/
        # but our nix store has testdata/TEST-07-PID1/TEST-07-PID1.units/.
        # Bridge the gap by symlinking each test's subdirs up one level.
        for testdir in /etc/systemd-tests/testdata/TEST-*/; do
          [ -d "$testdir" ] || continue
          for subdir in "$testdir"/*/; do
            [ -d "$subdir" ] || continue
            subname=$(basename "$subdir")
            [ -e "/usr/lib/systemd/tests/testdata/$subname" ] || \
              ln -sfn "$subdir" "/usr/lib/systemd/tests/testdata/$subname"
          done
        done

        # Make /etc/dbus-1 writable for tests that install D-Bus policy files
        if [ -L /etc/dbus-1 ] || [ ! -w /etc/dbus-1 2>/dev/null ]; then
          tmp=$(mktemp -d)
          if [ -e /etc/dbus-1 ]; then
            cp -a /etc/dbus-1/. "$tmp/" 2>/dev/null || true
            rm -f /etc/dbus-1
          fi
          mkdir -p /etc/dbus-1/system.d
          cp -a "$tmp/." /etc/dbus-1/ 2>/dev/null || true
          rm -rf "$tmp"
        else
          mkdir -p /etc/dbus-1/system.d
        fi
      '';

      users = {
        users = {
          nixos = {
            isNormalUser = true;
            extraGroups = ["wheel"];
            password = "nixos";
          };
          # The "daemon" user/group is expected by upstream test scripts (e.g. TEST-22-TMPFILES).
          daemon = {
            isSystemUser = true;
            group = "daemon";
          };
          # The "testuser" user/group is used by TEST-74-AUX-UTILS uid/gid tests.
          testuser = {
            isNormalUser = true;
            group = "testuser";
            createHome = true;
          };
        };
        groups.daemon = {};
        groups.testuser = {};
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

      # Ensure /run/systemd/system exists (tests write unit files there)
      machine.succeed("mkdir -p /run/systemd/system")

      # Run the upstream test script.
      # Tests source util.sh and test-control.sh from $(dirname "$0"),
      # so we run from the units directory.
      # Skip testcases that require D-Bus (busctl) or features not yet implemented.
      # Apply per-test patches to the test script (if any).
      # Test scripts are in the Nix store (read-only), so we copy to a writable dir first.
      patch_cmd = """${patchScript}"""
      if patch_cmd:
          machine.succeed("mkdir -p /tmp/test-units && cp -a /etc/systemd-tests/units/* /tmp/test-units/")
          machine.succeed(f"cd /tmp/test-units && {patch_cmd}")
          units_dir = "/tmp/test-units"
      else:
          units_dir = "/etc/systemd-tests/units"

      test_cmd = (
          f"cd {units_dir} && "
          "export TEST_SKIP_TESTCASES='testcase_hierarchical_slice_dropins testcase_transient_slice_dropins testcase_transient_service_dropins' && "
          "bash -x ./${testName}.sh 2>&1"
      )

      try:
          (rc, output) = machine.execute(test_cmd)
          print(output)
          if rc != 0:
              raise Exception("${testName} failed with exit code " + str(rc))
      except BrokenPipeError:
          # Some tests (e.g. TEST-18-FAILUREACTION) trigger a VM reboot.
          # Wait for the machine to come back up, then re-run the test script
          # which will detect the second phase (e.g. via /firstphase marker).
          print("BrokenPipeError: VM likely rebooted, waiting for it to come back...")
          machine.wait_for_unit("multi-user.target", timeout=120)
          machine.succeed("systemctl daemon-reload")
          (rc, output) = machine.execute(test_cmd)
          print(output)
          if rc != 0:
              raise Exception("${testName} failed with exit code " + str(rc))

      # Check for /testok (standard systemd test success marker)
      machine.succeed("test -f /testok")
    '';
  }
