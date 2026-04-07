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
  testEnv ? {},
  testTimeout ? 1800,
  useUpstreamSystemd ? false,
}: let
  systemdSrc = pkgs.systemd.src;

  # Build a derivation containing all test scripts and testdata from upstream
  testScripts = pkgs.runCommand "systemd-test-scripts" {} ''
    mkdir -p $out/{units,testdata}

    # Copy test runner scripts (test/units/)
    cp -a ${systemdSrc}/test/units/* $out/units/

    # Copy integration test data (unit files, helper scripts, etc.)
    cp -r --no-preserve=mode ${systemdSrc}/test/testdata/integration-tests/* $out/testdata/

    # Copy additional testdata directories needed by subtests
    # (e.g. test-journals for TEST-04-JOURNAL corrupted-journals subtest)
    for d in ${systemdSrc}/test/testdata/*/; do
      name=$(basename "$d")
      [ "$name" = "integration-tests" ] && continue
      cp -r --no-preserve=mode "$d" "$out/testdata/$name"
    done

    # Restore execute bits on scripts (cp --no-preserve=mode strips them)
    find $out -name '*.sh' -o -name '*.py' | xargs chmod +x 2>/dev/null || true
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

      # Use rust-systemd as the systemd package (or upstream C systemd for baseline)
      systemd.package =
        if useUpstreamSystemd
        then pkgs.systemd
        else rustSystemdPackage;
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
        # Extend systemd's default executable search path to include /usr/bin
        # and /bin where we symlink NixOS tools. This is needed for unit files
        # using bare command names like "bash" in ExecStart=.
        managerEnvironment.PATH = lib.mkForce "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/run/current-system/sw/bin:/run/wrappers/bin";

        # Set DefaultEnvironment so service processes (and scripts they run)
        # can find common binaries in /usr/bin and /bin. Without this, bare
        # commands like "mkdir" in test helper scripts fail because NixOS
        # compiles systemd with DEFAULT_PATH pointing only to its own bin.
        settings.Manager.DefaultEnvironment = ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/run/current-system/sw/bin:/run/wrappers/bin"];

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
          # Use Type=simple so the service is immediately active.
          # ExecReload uses a token-based handshake: it writes a unique
          # token to reload-request, sends SIGHUP to both the PID file
          # daemon and $MAINPID, then waits for reload-done to contain
          # the same token.  This avoids races where spurious SIGHUPs
          # create a stale reload-done before the intended reload runs.
          systemd-journald.serviceConfig = {
            Type = lib.mkForce "simple";
            ExecReload = let
              reloadScript = pkgs.writeShellScript "journald-reload" ''
                REQ_FILE=/run/systemd/journal/reload-request
                DONE_FILE=/run/systemd/journal/reload-done
                PID_FILE=/run/systemd/journal/pid
                TOKEN="$$-$(date +%s%N)"
                echo "$TOKEN" > "$REQ_FILE"
                FILE_PID=""
                [ -f "$PID_FILE" ] && FILE_PID="$(cat "$PID_FILE" 2>/dev/null)"
                [ -n "$FILE_PID" ] && kill -HUP "$FILE_PID" 2>/dev/null || true
                if [ -n "''${MAINPID:-}" ] && [ "''${MAINPID:-}" != "$FILE_PID" ]; then
                  kill -HUP "''${MAINPID}" 2>/dev/null || true
                fi
                [ -z "$FILE_PID" ] && [ -z "''${MAINPID:-}" ] && exit 0
                i=0
                while [ "$i" -lt 600 ]; do
                  DONE_CONTENT="$(cat "$DONE_FILE" 2>/dev/null)"
                  [ "$DONE_CONTENT" = "$TOKEN" ] && exit 0
                  sleep 0.05
                  i=$((i + 1))
                done
                echo "journald: reload timed out waiting for token $TOKEN" >&2
                exit 1
              '';
            in
              lib.mkForce ["${reloadScript}"];
          };
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
            python3 # for TEST-04-JOURNAL LogFilterPatterns syslog test
            iproute2
            util-linux
            jq
            procps
            kmod
            hostname # for hostname command
            acl # for setfacl/getfacl
            zstd # for unzstd (e.g. TEST-04-JOURNAL corrupted-journals)
          ]
          ++ extraPackages;
      };

      # Install test-specific unit files into systemd unit path if they exist
      system.activationScripts.systemd-test-units = ''
        # Always create /run/systemd/system so tests can write unit files there
        mkdir -p /run/systemd/system

        # Symlink common binaries to /usr/bin and /bin so systemd unit
        # ExecStart= lines using bare command names work on NixOS where
        # everything lives in /nix/store. Use readlink -f to resolve symlink
        # chains (e.g. sh -> bash) to avoid broken chains.
        mkdir -p /usr/bin /bin /usr/sbin /sbin
        for dir in ${pkgs.bash}/bin ${pkgs.coreutils}/bin ${pkgs.gnugrep}/bin ${pkgs.gnused}/bin ${pkgs.findutils}/bin ${pkgs.gawk}/bin ${pkgs.diffutils}/bin ${pkgs.util-linux}/bin ${pkgs.procps}/bin ${pkgs.kmod}/bin ${pkgs.iproute2}/bin ${pkgs.jq}/bin ${pkgs.hostname}/bin ${pkgs.python3}/bin ${config.systemd.package}/bin; do
          [ -d "$dir" ] || continue
          for bin in "$dir"/*; do
            [ -x "$bin" ] || continue
            name=$(basename "$bin")
            # Resolve to final target to avoid dangling symlink chains
            real=$(readlink -f "$bin" 2>/dev/null) || real="$bin"
            [ -e "/usr/bin/$name" ] || ln -sfn "$real" "/usr/bin/$name"
            [ -e "/bin/$name" ] || ln -sfn "$real" "/bin/$name"
          done
        done

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
                cp "$f" "/run/systemd/system/$name"
                chmod 644 "/run/systemd/system/$name"
                ;;
              *.sh)
                # Copy helper scripts preserving executability
                cp "$f" "/run/systemd/system/$name"
                chmod +x "/run/systemd/system/$name"
                ;;
              *.wants|*.requires)
                cp -a "$f" "/run/systemd/system/$name"
                ;;
            esac
          done
        fi

        # Create writable /usr/lib/systemd/system/ for tests that write units there.
        # Also symlink upstream systemd unit files that tests reference (e.g.
        # systemd-importd.service for TEST-03-JOBS).  We don't symlink ALL
        # upstream units because loading hundreds of extra units at boot can
        # overwhelm PID 1.
        mkdir -p /usr/lib/systemd/system
        for f in ${config.systemd.package}/example/systemd/system/systemd-importd.service \
                 ${config.systemd.package}/example/systemd/system/systemd-journald@.service \
                 ${config.systemd.package}/example/systemd/system/systemd-journald@.socket \
                 ${config.systemd.package}/example/systemd/system/systemd-journald-varlink@.socket; do
          name=$(basename "$f")
          [ -e "/usr/lib/systemd/system/$name" ] || ln -sfn "$f" "/usr/lib/systemd/system/$name"
        done

        # Symlink helper binaries (systemd-sysctl, etc.) so tests referencing
        # /usr/lib/systemd/systemd-* can find them (NixOS puts them in the store)
        for bin in ${config.systemd.package}/lib/systemd/systemd-*; do
          name=$(basename "$bin")
          [ -e "/usr/lib/systemd/$name" ] || ln -sfn "$bin" "/usr/lib/systemd/$name"
        done

        # Symlink generator directories so tests referencing
        # /usr/lib/systemd/system-generators/ and /usr/lib/systemd/user-environment-generators/
        # can find generator binaries (e.g. TEST-81-GENERATORS)
        for gendir in system-generators user-environment-generators user-generators; do
          if [ -d "${config.systemd.package}/lib/systemd/$gendir" ]; then
            ln -sfn "${config.systemd.package}/lib/systemd/$gendir" "/usr/lib/systemd/$gendir"
          fi
        done

        # Symlink manual test binaries (e.g. test-journal-append)
        if [ -d "${config.systemd.package}/lib/systemd/tests/unit-tests/manual" ]; then
          mkdir -p /usr/lib/systemd/tests/unit-tests
          ln -sfn "${config.systemd.package}/lib/systemd/tests/unit-tests/manual" \
            /usr/lib/systemd/tests/unit-tests/manual
        fi

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
        # Bridge the gap by copying (not symlinking) each test's subdirs up one
        # level so we can patch bare command names in ExecStart= lines below.
        for testdir in /etc/systemd-tests/testdata/TEST-*/; do
          [ -d "$testdir" ] || continue
          for subdir in "$testdir"/*/; do
            [ -d "$subdir" ] || continue
            subname=$(basename "$subdir")
            if [ ! -e "/usr/lib/systemd/tests/testdata/$subname" ]; then
              cp -r --no-preserve=mode "$subdir" "/usr/lib/systemd/tests/testdata/$subname"
              # Restore execute permission on scripts (cp --no-preserve=mode strips it)
              ${pkgs.findutils}/bin/find "/usr/lib/systemd/tests/testdata/$subname" \
                \( -name '*.sh' -o -name '*.py' \) -type f -exec chmod +x {} +
              # Add missing shebangs to Python scripts (upstream relies on
              # binfmt_misc or build-system patching; NixOS has neither)
              for pyf in $(${pkgs.findutils}/bin/find "/usr/lib/systemd/tests/testdata/$subname" -name '*.py' -type f 2>/dev/null); do
                if ! head -1 "$pyf" | grep -q '^#!'; then
                  ${pkgs.gnused}/bin/sed -i "1i#!/usr/bin/python3" "$pyf"
                fi
              done
            fi
          done
        done

        # NixOS compiles systemd with DEFAULT_PATH_NORMAL pointing only to the
        # systemd package's own bin directory. The systemd-executor uses this
        # compiled-in path to resolve bare command names in ExecStart= lines,
        # so upstream test unit files that use e.g. "ExecStart=bash ..." fail
        # with "Unable to locate executable". Patch all .service files in the
        # writable testdata and /run/systemd/system to use absolute paths.
        # Use full nix store paths for find/sed since activation scripts have
        # a limited PATH.
        patch_unit_bare_commands() {
          local dir="$1"
          ${pkgs.findutils}/bin/find "$dir" -name '*.service' -type f 2>/dev/null | while read -r svc; do
            ${pkgs.gnused}/bin/sed -i \
              -e 's|^ExecStart=bash |ExecStart=/usr/bin/bash |' \
              -e 's|^ExecStart=sh |ExecStart=/bin/sh |' \
              -e 's|^ExecStart=cat |ExecStart=/usr/bin/cat |' \
              -e 's|^ExecStart=echo |ExecStart=/usr/bin/echo |' \
              -e 's|^ExecStart=sleep |ExecStart=/usr/bin/sleep |' \
              -e 's|^ExecStart=true|ExecStart=/usr/bin/true|' \
              -e 's|^ExecStart=false|ExecStart=/usr/bin/false|' \
              -e 's|^ExecStop=bash |ExecStop=/usr/bin/bash |' \
              -e 's|^ExecStop=sh |ExecStop=/bin/sh |' \
              -e 's|^ExecReload=bash |ExecReload=/usr/bin/bash |' \
              -e 's|^ExecStartPre=bash |ExecStartPre=/usr/bin/bash |' \
              -e 's|^ExecStartPost=bash |ExecStartPost=/usr/bin/bash |' \
              "$svc"
          done
        }
        patch_unit_bare_commands /usr/lib/systemd/tests/testdata
        patch_unit_bare_commands /run/systemd/system

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

      # Ensure scripts in testdata are executable (activation script may not
      # always set the execute bit correctly due to nix store copy semantics)
      machine.succeed("find /usr/lib/systemd/tests/testdata -name '*.sh' -o -name '*.py' | xargs chmod +x 2>/dev/null || true")
      machine.succeed("find /run/systemd/system -name '*.sh' -o -name '*.py' | xargs chmod +x 2>/dev/null || true")

      # Ensure /run/systemd/system exists (tests write unit files there)
      machine.succeed("mkdir -p /run/systemd/system")

      # Run the upstream test script.
      # Tests source util.sh and test-control.sh from $(dirname "$0"),
      # so we run from the units directory.
      # Skip testcases that require D-Bus (busctl) or features not yet implemented.
      # Apply per-test patches to the test script (if any).
      # Test scripts are in the Nix store (read-only), so we copy to a writable dir first.
      # Always copy to writable dir: scripts need +x for direct exec and
      # the Nix store source is read-only.
      machine.succeed("mkdir -p /tmp/test-units && cp -a /etc/systemd-tests/units/* /tmp/test-units/")
      patch_cmd = """${patchScript}"""
      if patch_cmd:
          machine.succeed(f"cd /tmp/test-units && {patch_cmd}")
      units_dir = "/tmp/test-units"

      env_exports = "${builtins.concatStringsSep "; " (builtins.attrValues (builtins.mapAttrs (k: v: "export ${k}='${v}'") testEnv))}"
      env_prefix = f"{env_exports}; " if env_exports else ""
      # Exec the script directly (not via `bash -x`) so the kernel sets
      # /proc/PID/comm to the script filename.  This is needed for
      # `journalctl -b "$(readlink -f "$0")"` (script-as-path matching)
      # which checks _COMM against the script basename.  The upstream test
      # scripts already set `set -eux` internally.
      # Tee output to /dev/kmsg so it appears on serial console (nix build -L).
      # Use a FIFO to capture the exit code without PIPESTATUS (avoiding Nix escaping).
      test_cmd = f"cd {units_dir} && {env_prefix}chmod +x ./${testName}.sh && ./${testName}.sh 2>&1 | tee /dev/ttyS0"

      try:
          (rc, output) = machine.execute(test_cmd, timeout=${toString testTimeout})
          print(output)
          # tee masks the real exit code; rely on /testok check below
      except BrokenPipeError:
          # Some tests (e.g. TEST-18-FAILUREACTION) trigger a VM reboot.
          # Wait for the machine to come back up, then re-run the test script
          # which will detect the second phase (e.g. via /firstphase marker).
          print("BrokenPipeError: VM likely rebooted, waiting for it to come back...")
          machine.wait_for_unit("multi-user.target", timeout=120)
          machine.succeed("systemctl daemon-reload")
          (rc, output) = machine.execute(test_cmd, timeout=${toString testTimeout})
          print(output)

      # Check for /testok (standard systemd test success marker)
      machine.succeed("test -f /testok")
    '';
  }
