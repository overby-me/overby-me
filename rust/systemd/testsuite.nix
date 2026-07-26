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
  # Attach a software TPM (swtpm -> /dev/tpmrm0) to the VM. Needed by TEST-70-TPM2,
  # whose systemd-tpm2-setup/tpm2 tooling requires a TPM2 device.
  enableTpm ? false,
  # Start the VM with allow_reboot=True so a guest `systemctl reboot` restarts
  # the VM (QEMU resets + re-runs the same kernel) instead of the default
  # -no-reboot behaviour (guest reboot terminates QEMU). Needed by reboot tests
  # (TEST-09-REBOOT); harmless but undesirable for others (a kernel panic would
  # loop instead of fast-failing), so it is opt-in per test.
  allowReboot ? false,
  # Accept `/skipped` (upstream's exit-77 convention) as a passing outcome.
  # OFF by default: a skip means zero assertions ran, so it fails the check and
  # a newly-self-skipping test surfaces as red.  Every test that sets this is
  # listed in docs/TEST-OVERRIDES.md with what it would take to unset it.
  expectedSkip ? false,
  # Extra unit files to link into /usr/lib/systemd/system from the C systemd
  # package, by basename (e.g. "systemd-oomd.service").  rust-systemd overlays
  # its binaries onto that package, so the units ship with it but only a small
  # default set is linked; linking all of them at boot is wasteful.  Tests that
  # exercise a specific daemon name the units they need here.
  extraUnits ? [],
  # Boot the VM through a real bootloader from a disk image (instead of the
  # default directBoot, which loads the kernel directly and cannot cleanly
  # re-run the whole boot on reboot). A firmware reboot re-initialises every
  # device and re-runs the full boot, so the test driver's backdoor console is
  # re-established exactly like the first boot. Needed by multi-boot reboot
  # tests; implies allowReboot. Opt-in per test (slower: builds a disk image).
  useBootLoader ? false,
}: let
  systemdSrc = pkgs.systemd.src;

  # Write the patchScript to a real file to avoid Python string interpolation
  # mangling escape sequences like \n, \t inside heredocs.
  patchScriptFile = pkgs.writeShellScript "patch-test.sh" ''
    set -eu
    ${patchScript}
  '';

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
    inherit (pkgs) rust-systemd;
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
        # Copy rules that either:
        #  * reference `systemctl` (systemd-unit integrations)
        #  * ship builtin IMPORTs (`net_id`, `net_setup_link`, …) needed
        #    by TEST-17-UDEV.* for ID_NET_DRIVER/ID_NET_NAME-style props
        #  * 99-systemd.rules (required verbatim by the sanity-check
        #    subtest's `udevadm cat 99-systemd`)
        for rule in ${config.systemd.package}/lib/udev/rules.d/*.rules; do
          name=$(basename "$rule")
          if grep -q 'systemctl' "$rule" \
             || grep -q 'IMPORT{builtin}' "$rule" \
             || [ "$name" = "99-systemd.rules" ]; then
            # Skip rules that IMPORT{program}= a relative-path helper the
            # systemd package doesn't ship as a standalone binary — e.g.
            # 60-tpm2-id.rules calls `tpm2_id`, whose logic now lives only in
            # the udevadm builtin, not a program file. NixOS's udev-rules
            # builder rejects such references ("<prog> is called in udev rules
            # but not installed by udev"), which would fail the whole system
            # build. NixOS itself ships none of these rules and no integration
            # test needs them, so drop them here.
            missing=""
            for prog in $(sed -nE 's/.*IMPORT\{program\}="([^/$][^ "]*).*/\1/p' "$rule"); do
              [ -x "${config.systemd.package}/lib/udev/$prog" ] || missing=1
            done
            [ -n "$missing" ] && continue
            cp "$rule" "$out/lib/udev/rules.d/$name"
          fi
        done
      '';
    in {
      system.stateVersion = "25.11";

      # Boot the traditional bash stage-1 initrd rather than systemd-in-initrd.
      #
      # Newer nixpkgs flipped `boot.initrd.systemd.enable` to default true, which
      # makes the *initramfs* PID 1 be the systemd binary (here rust-systemd).
      # rust-systemd implements the stage-2 system manager, not the initrd
      # stage-1 (mount API filesystems, mount /sysroot, switch_root, exec
      # stage-2), so it cannot yet run as the initrd init. The bash stage-1
      # initrd mounts the API filesystems and switch_roots into rust-systemd as
      # the stage-2 manager — the role it is designed for, and the setup these
      # integration tests exercise. Re-enabling systemd-in-initrd is tracked as
      # future work once rust-systemd grows an initrd mode.  This is also why
      # TEST-08-INITRD is expectedSkip: it bails out when
      # InitRDTimestampMonotonic is 0.
      boot.initrd.systemd.enable = lib.mkIf (!useUpstreamSystemd) (lib.mkForce false);

      # Use rust-systemd as the systemd package (or upstream C systemd for baseline)
      systemd.package =
        if useUpstreamSystemd
        then pkgs.systemd
        else rustSystemdPackage;
      services.udev.packages = [udevRulesOverride];

      # Replace the upstream bash stage-2-init.sh with a version that
      # strips the racy `exec > >(tee -i /proc/self/fd/"$logOutFd" |
      # while read -r line; …) 2>&1` process-substitution pipeline.
      # That bash-level fd-inheritance setup races with parallel
      # kernel module loading (fuse/vmci/vsock auto-load) during
      # early boot and hangs the VM about 30% of the time.  Taking
      # the pipe out means stage-2 output goes straight to /dev/console
      # instead of being re-logged to /dev/kmsg by a subshell, which
      # is a small loss of diagnostic polish for a large boot-stability
      # win.  All other stage-2 behavior (activation script, exec
      # systemd) is preserved verbatim.
      system.build.bootStage2 = let
        # Delete the whole `if test -w /dev/kmsg; then ... fi` block
        # that wraps the racy `exec > >(tee | while read) 2>&1` pipe.
        # Deleting only the inner then-branch would leave bash with an
        # empty `then`, which is a syntax error.  Stripping the entire
        # if/fi chain makes stage-2 output go directly to inherited
        # stdout (/dev/console from stage-1) — no subshell, no pipe,
        # no race.
        patchedSrc = pkgs.runCommand "stage-2-init-no-tee-pipe.sh" {} ''
          # End pattern `^    fi$` (4 leading spaces, nothing else)
          # deliberately matches the outer `fi` at column 4 closing
          # the `if test -w /dev/kmsg; then … fi` block — not the
          # deeper 12-space inner `fi` that closes `if test -n "$line"`.
          sed '/^ *if test -w \/dev\/kmsg; then$/,/^    fi$/d' \
              ${pkgs.path}/nixos/modules/system/boot/stage-2-init.sh \
              > $out
        '';
      in
        lib.mkIf (!useUpstreamSystemd) (
          lib.mkForce (pkgs.replaceVarsWith {
            src = patchedSrc;
            isExecutable = true;
            replacements = {
              shell = "${pkgs.bash}/bin/bash";
              systemConfig = null;
              inherit (config.boot) systemdExecutable;
              nixStoreMountOpts = lib.concatStringsSep " " (
                map lib.escapeShellArg config.boot.nixStoreMountOpts
              );
              # Mirror the stage-2 module's own `inherit (config.boot) …
              # stage2Greeting`. Newer nixpkgs replaced the `@distroName@`
              # placeholder with a precomputed `@stage2Greeting@`, so passing
              # distroName now makes the strict replaceVarsWith fail ("pattern
              # @distroName@ doesn't match anything").
              inherit (config.boot) stage2Greeting;
              useHostResolvConf =
                config.networking.resolvconf.enable
                && config.networking.useHostResolvConf;
              inherit (config.system.build) earlyMountScript;
              path = lib.makeBinPath [pkgs.coreutils pkgs.util-linux];
              postBootCommands = pkgs.writeText "local-cmds" ''
                ${config.boot.postBootCommands}
                ${config.powerManagement.powerUpCommands}
              '';
            };
          })
        );

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
          # The rust-systemd package bundles the upstream C systemd-timesyncd
          # binary but doesn't reimplement it, so NixOS generates an
          # Environment-only unit with no ExecStart/Type — which rust correctly
          # parses as a completed oneshot (ActiveState=inactive).  Supply
          # ExecStart/Type so `timedatectl set-ntp true` brings timesyncd active
          # (TEST-45-TIMEDATE testcase_ntp asserts assert_timesyncd_state active).
          systemd-timesyncd.serviceConfig = {
            PrivateDevices = lib.mkForce false;
            Type = lib.mkForce "notify";
            ExecStart =
              lib.mkForce "${config.systemd.package}/lib/systemd/systemd-timesyncd";
            StateDirectory = lib.mkForce "systemd/timesync";
            RuntimeDirectory = lib.mkForce "systemd/timesync";
          };
          # Use Type=simple so the service is immediately active.
          # ExecReload uses a token-based handshake: it writes a unique
          # token to reload-request, sends SIGHUP to both the PID file
          # daemon and $MAINPID, then waits for reload-done to contain
          # the same token.  This avoids races where spurious SIGHUPs
          # create a stale reload-done before the intended reload runs.
          systemd-journald.serviceConfig = {
            Type = lib.mkForce "simple";
            FileDescriptorStoreMax = lib.mkDefault 4224;
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
          # Override systemd-journal-upload — NixOS includes it even
          # without `services.journald.upload.enable = true`, and since
          # no URL is configured it immediately exits with "No URL
          # specified", leaving `systemctl is-system-running` at
          # "degraded" and tripping the TEST-74-AUX-UTILS
          # is-system-running test.  Clear wantedBy so nothing pulls
          # it in on default.target, and replace ExecStart with
          # `/bin/true` so any explicit start call is a no-op.
          systemd-journal-upload = {
            wantedBy = lib.mkForce [];
            serviceConfig.ExecStart = lib.mkForce "${pkgs.coreutils}/bin/true";
            serviceConfig.Restart = lib.mkForce "no";
          };
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

      environment = {
        # Install test scripts and testdata into the VM
        etc = {
          "systemd-tests/units".source = "${testScripts}/units";
          "systemd-tests/testdata".source = "${testScripts}/testdata";
          "systemd-tests/patch-test.sh".source = "${patchScriptFile}";
        };

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
        # Writable network config dir for tests that write .network/.netdev/.link
        # files under /usr/lib/systemd/network/ (TEST-74-AUX-UTILS.networkctl.sh).
        mkdir -p /usr/lib/systemd/network
        for f in ${config.systemd.package}/example/systemd/system/systemd-importd.service \
                 ${config.systemd.package}/example/systemd/system/systemd-journald@.service \
                 ${config.systemd.package}/example/systemd/system/systemd-journald@.socket \
                 ${config.systemd.package}/example/systemd/system/systemd-journald-varlink@.socket \
                 ${config.systemd.package}/example/systemd/system/systemd-sysusers.service \
                 ${config.systemd.package}/lib/systemd/system/systemd-journal-gatewayd.service \
                 ${config.systemd.package}/lib/systemd/system/systemd-journal-gatewayd.socket \
                 ${config.systemd.package}/lib/systemd/system/systemd-bsod.service; do
          name=$(basename "$f")
          [ -e "/usr/lib/systemd/system/$name" ] || ln -sfn "$f" "/usr/lib/systemd/system/$name"
        done

        # Per-test extra units (the `extraUnits` argument).  The C systemd
        # package keeps most of its optional units under example/systemd/system
        # rather than lib/systemd/system, so look in both.  Fail loudly on a
        # typo: a silently missing unit turns into a confusing "unit not found"
        # deep inside the test.
        ${pkgs.lib.optionalString (extraUnits != []) ''
          for name in ${pkgs.lib.escapeShellArgs extraUnits}; do
            [ -e "/usr/lib/systemd/system/$name" ] && continue
            found=""
            for dir in ${config.systemd.package}/example/systemd/system \
                       ${config.systemd.package}/lib/systemd/system; do
              if [ -e "$dir/$name" ]; then
                ln -sfn "$dir/$name" "/usr/lib/systemd/system/$name"
                found=1
                break
              fi
            done
            if [ -z "$found" ]; then
              echo "extraUnits: no such unit '$name' in the systemd package" >&2
              exit 1
            fi
          done
        ''}

        # The rust-systemd package doesn't ship systemd-network-generator.service
        # in its unit dir, so provide it inline for TEST-74-AUX-UTILS.network-generator.
        # No ConditionKernelCommandLine so the credential-only path still runs the
        # generator; the binary is symlinked into /usr/lib/systemd/ just below.
        if [ ! -e /usr/lib/systemd/system/systemd-network-generator.service ]; then
          printf '%s\n' \
            '[Unit]' \
            'Description=Generate network units from Kernel command line and credentials' \
            'DefaultDependencies=no' \
            'Before=network-pre.target' \
            '[Service]' \
            'Type=oneshot' \
            'RemainAfterExit=yes' \
            'ExecStart=/usr/lib/systemd/systemd-network-generator' \
            > /usr/lib/systemd/system/systemd-network-generator.service
        fi

        # The rust-systemd package doesn't ship systemd-ask-password.socket/.service,
        # so provide them inline for TEST-74-AUX-UTILS.ask-password: the varlink
        # introspect of /run/systemd/io.systemd.AskPassword needs a listening socket
        # that activates systemd-ask-password in its Varlink server mode (LISTEN_FDS).
        if [ ! -e /usr/lib/systemd/system/systemd-ask-password.socket ]; then
          printf '%s\n' \
            '[Unit]' \
            'Description=Query the User Interactively for a Password' \
            'DefaultDependencies=no' \
            'Before=sockets.target' \
            '[Socket]' \
            'ListenStream=/run/systemd/io.systemd.AskPassword' \
            'SocketMode=0666' \
            '[Install]' \
            'WantedBy=sockets.target' \
            > /usr/lib/systemd/system/systemd-ask-password.socket
          printf '%s\n' \
            '[Unit]' \
            'Description=Query the User Interactively for a Password' \
            'DefaultDependencies=no' \
            '[Service]' \
            'ExecStart=-/usr/bin/systemd-ask-password --no-tty' \
            > /usr/lib/systemd/system/systemd-ask-password.service
          mkdir -p /usr/lib/systemd/system/sockets.target.wants
          ln -sfn ../systemd-ask-password.socket /usr/lib/systemd/system/sockets.target.wants/systemd-ask-password.socket
        fi

        # The rust-systemd package doesn't enable systemd-mute-console.socket by
        # default, so provide it inline for TEST-74-AUX-UTILS.mute-console: the
        # introspect/call of /run/systemd/io.systemd.MuteConsole needs a listening
        # Accept=yes varlink socket that spawns systemd-mute-console per connection
        # (the accepted connection is passed as fd 3 via LISTEN_FDS).
        if [ ! -e /usr/lib/systemd/system/systemd-mute-console.socket ]; then
          printf '%s\n' \
            '[Unit]' \
            'Description=Console Output Muting Service Socket' \
            'DefaultDependencies=no' \
            'Before=sockets.target' \
            '[Socket]' \
            'ListenStream=/run/systemd/io.systemd.MuteConsole' \
            'FileDescriptorName=varlink' \
            'SocketMode=0600' \
            'Accept=yes' \
            'MaxConnectionsPerSource=16' \
            '[Install]' \
            'WantedBy=sockets.target' \
            > /usr/lib/systemd/system/systemd-mute-console.socket
          printf '%s\n' \
            '[Unit]' \
            'Description=Console Output Muting Service' \
            'DefaultDependencies=no' \
            '[Service]' \
            'ExecStart=/usr/bin/systemd-mute-console' \
            > /usr/lib/systemd/system/systemd-mute-console@.service
          mkdir -p /usr/lib/systemd/system/sockets.target.wants
          ln -sfn ../systemd-mute-console.socket /usr/lib/systemd/system/sockets.target.wants/systemd-mute-console.socket
        fi

        # The rust-systemd package doesn't enable systemd-journalctl.socket by
        # default, so provide it inline for TEST-04-JOURNAL.journalctl-varlink:
        # the introspect/call of /run/systemd/io.systemd.JournalAccess needs a
        # listening Accept=yes varlink socket that spawns journalctl per
        # connection (the accepted connection is passed as fd 3 via LISTEN_FDS,
        # and journalctl serves io.systemd.JournalAccess.GetEntries).
        if [ ! -e /usr/lib/systemd/system/systemd-journalctl.socket ]; then
          printf '%s\n' \
            '[Unit]' \
            'Description=Journal Log Access Socket' \
            'DefaultDependencies=no' \
            'Before=sockets.target' \
            '[Socket]' \
            'ListenStream=/run/systemd/io.systemd.JournalAccess' \
            'FileDescriptorName=varlink' \
            'SocketMode=0666' \
            'Accept=yes' \
            'MaxConnectionsPerSource=16' \
            '[Install]' \
            'WantedBy=sockets.target' \
            > /usr/lib/systemd/system/systemd-journalctl.socket
          printf '%s\n' \
            '[Unit]' \
            'Description=Journal Log Access' \
            'DefaultDependencies=no' \
            '[Service]' \
            'ExecStart=/usr/bin/journalctl' \
            > /usr/lib/systemd/system/systemd-journalctl@.service
          mkdir -p /usr/lib/systemd/system/sockets.target.wants
          ln -sfn ../systemd-journalctl.socket /usr/lib/systemd/system/sockets.target.wants/systemd-journalctl.socket
        fi

        # Symlink helper binaries (systemd-sysctl, etc.) so tests referencing
        # /usr/lib/systemd/systemd-* can find them (NixOS puts them in the store)
        for bin in ${config.systemd.package}/lib/systemd/systemd-*; do
          name=$(basename "$bin")
          [ -e "/usr/lib/systemd/$name" ] || ln -sfn "$bin" "/usr/lib/systemd/$name"
        done

        # Symlink udev rules files so tests like TEST-17-UDEV.sanity-check.sh
        # that reference /usr/lib/udev/rules.d/99-systemd.rules directly can
        # find them (NixOS installs rules only under /etc/udev/rules.d).
        mkdir -p /usr/lib/udev/rules.d
        for f in ${config.systemd.package}/lib/udev/rules.d/*.rules; do
          [ -e "$f" ] || continue
          name=$(basename "$f")
          [ -e "/usr/lib/udev/rules.d/$name" ] || ln -sfn "$f" "/usr/lib/udev/rules.d/$name"
        done

        # Populate /dev/block/ with major:minor symlinks for existing block
        # devices. The TEST-17-UDEV.sanity-check.sh lock-test iterates
        # `/dev/block/*` and set-e aborts if the glob is empty — this ensures
        # at least one entry is present even if rust-udevd hasn't processed
        # block events yet.
        mkdir -p /dev/block
        for blk in /sys/block/*; do
          [ -d "$blk" ] || continue
          if [ -f "$blk/dev" ]; then
            devstr=$(cat "$blk/dev" 2>/dev/null)
            name=$(basename "$blk")
            if [ -n "$devstr" ] && [ -e "/dev/$name" ]; then
              [ -e "/dev/block/$devstr" ] || ln -sfn "/dev/$name" "/dev/block/$devstr"
            fi
          fi
        done

        # Copy share data (e.g. gatewayd/browse.html) to /usr/share as a
        # writable directory.  Tests like journal-gatewayd need to mv/restore
        # files under /usr/share/systemd, which fails if it's a read-only
        # symlink into the Nix store.
        if [ -d "${config.systemd.package}/share/systemd" ]; then
          mkdir -p /usr/share
          cp -a "${config.systemd.package}/share/systemd" /usr/share/systemd
          chmod -R u+w /usr/share/systemd
        fi

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
              -e 's|^ExecStartPre=true|ExecStartPre=/usr/bin/true|' \
              -e 's|^ExecStartPre=echo |ExecStartPre=/usr/bin/echo |' \
              -e 's|^ExecStartPre=sleep |ExecStartPre=/usr/bin/sleep |' \
              -e 's|^ExecStartPre=grep |ExecStartPre=/usr/bin/grep |' \
              -e 's|^ExecStartPre=test |ExecStartPre=/usr/bin/test |' \
              -e 's|^ExecStart=grep |ExecStart=/usr/bin/grep |' \
              -e 's|^ExecStart=test |ExecStart=/usr/bin/test |' \
              -e 's|^ExecStart=touch |ExecStart=/usr/bin/touch |' \
              -e 's|^ExecStart=mkdir |ExecStart=/usr/bin/mkdir |' \
              -e 's|^ExecStart=cp |ExecStart=/usr/bin/cp |' \
              -e 's|^ExecStart=rm |ExecStart=/usr/bin/rm |' \
              -e 's|^ExecStartPost=bash |ExecStartPost=/usr/bin/bash |' \
              -e 's|^ExecStartPost=sh |ExecStartPost=/bin/sh |' \
              -e 's|^ExecStopPost=bash |ExecStopPost=/usr/bin/bash |' \
              -e 's|^ExecStopPost=touch |ExecStopPost=/usr/bin/touch |' \
              -e 's|^ExecStopPost=echo |ExecStopPost=/usr/bin/echo |' \
              -e 's|^ExecStopPost=sh |ExecStopPost=/bin/sh |' \
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
          # systemd-journal-remote user for journal-remote test
          systemd-journal-remote = {
            isSystemUser = true;
            group = "systemd-journal";
          };
          # systemd-timesync user for TEST-45-TIMEDATE: `timedatectl set-ntp true`
          # starts systemd-timesyncd, which runs as User=systemd-timesync (real
          # systemd ships example/sysusers.d/systemd-timesync.conf to create it).
          systemd-timesync = {
            isSystemUser = true;
            group = "systemd-timesync";
          };
        };
        groups.daemon = {};
        groups.testuser = {};
        groups.systemd-timesync = {};
      };

      # Give the VM enough resources for tests
      virtualisation =
        {
          # The integration-test manager is built from the debug dev profile
          # (rust-systemd-dev): its unoptimized PID 1 binary needs more RAM than
          # the release build, so give the VM headroom to avoid an early-boot OOM.
          memorySize = 4096;
          cores = 2;
          # Software TPM for TEST-70-TPM2 (opt-in per test via enableTpm).
          tpm.enable = enableTpm;
        }
        // pkgs.lib.optionalAttrs useBootLoader {
          # Boot via a bootloader on a disk image so a guest reboot re-runs the
          # full firmware->bootloader->kernel boot (see the useBootLoader arg).
          useBootLoader = true;
          useEFIBoot = true;
        };
      # A bootloader is required when useBootLoader is set; systemd-boot is
      # installed into the ESP at image-build time (using the build host's
      # systemd, so it does not depend on rust-systemd's own bootctl).
      boot.loader = pkgs.lib.mkIf useBootLoader {
        systemd-boot.enable = true;
        efi.canTouchEfiVariables = true;
        timeout = 0;
      };
    };

    testScript = ''
      ${pkgs.lib.optionalString (allowReboot || useBootLoader) "machine.start(allow_reboot=True)"}
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
      machine.succeed("cd /tmp/test-units && /etc/systemd-tests/patch-test.sh")
      units_dir = "/tmp/test-units"

      env_exports = "${pkgs.lib.concatStringsSep "; " (pkgs.lib.attrValues (pkgs.lib.mapAttrs (k: v: "export ${k}='${v}'") testEnv))}"
      env_prefix = f"{env_exports}; " if env_exports else ""
      # Exec the script directly (not via `bash -x`) so the kernel sets
      # /proc/PID/comm to the script filename.  This is needed for
      # `journalctl -b "$(readlink -f "$0")"` (script-as-path matching)
      # which checks _COMM against the script basename.  The upstream test
      # scripts already set `set -eux` internally.
      # Tee output to /dev/kmsg so it appears on serial console (nix build -L).
      # Use a FIFO to capture the exit code without PIPESTATUS (avoiding Nix escaping).
      # Wrap the test script so an exit code of 77 (upstream convention for
      # "prerequisite missing, skip") creates /skipped even when the script
      # didn't touch it itself (e.g. TEST-83-BTRFS exits 77 on non-btrfs root
      # without writing /skipped).
      # Run the test in its OWN session (setsid) so it is not a background
      # process group of the backdoor shell's controlling tty (/dev/hvc0).
      # Without this, util-linux `script`'s tcsetattr(STDIN, raw) generates
      # SIGTTOU and hangs forever (task #51: blocks networkctl-edit + pty-forward).
      # `-w` waits for the child so the /testok check below still works.
      test_cmd = f"cd {units_dir} && chmod +x *.sh && {env_prefix}setsid -w bash -c './${testName}.sh; rc=$?; [ $rc -eq 77 ] && touch /skipped; exit $rc' 2>&1 | tee /dev/ttyS0"

      # A test may reboot the VM one or more times (TEST-09-REBOOT's multi-boot
      # journal cycle, TEST-18-FAILUREACTION, ...). A reboot mid-execute tears
      # down the backdoor shell, surfacing as EITHER a BrokenPipeError (the
      # connection dropped) OR a binascii.Error (the reboot truncated the final
      # base64 output frame). On either, reconnect to the rebooted VM and re-run
      # the test script, which resumes the next phase itself (it tracks progress
      # across boots, e.g. via /var/tmp/.systemd_reboot_count). Loop until the
      # script completes without rebooting. Non-rebooting tests take the first
      # iteration and break immediately, so their behaviour is unchanged.
      import binascii
      reboot_seen = 0
      while True:
          try:
              (rc, output) = machine.execute(test_cmd, timeout=${toString testTimeout})
              print(output)
              # tee masks the real exit code; rely on /testok check below
              break
          except (BrokenPipeError, binascii.Error):
              reboot_seen += 1
              if reboot_seen > 10:
                  raise Exception("too many reboots; aborting to avoid an infinite loop")
              print(f"VM rebooted (#{reboot_seen}), reconnecting and resuming the test...")
              machine.connected = False
              machine.wait_for_unit("multi-user.target", timeout=120)
              machine.succeed("systemctl daemon-reload")

      # Check for /testok, the standard systemd test success marker.  Its
      # absence means the script crashed or a set-e-trapped `test` assertion
      # failed.
      #
      # /skipped is the upstream convention for a test that detected a missing
      # prerequisite and bailed out (exit 77).  A skip is NOT a pass: it means
      # zero assertions ran.  By default it fails the check, so a test that
      # starts self-skipping (a regressed prerequisite, a masked feature) shows
      # up as red instead of silently green.  Tests that are legitimately
      # expected to skip set `expectedSkip = true` in their integration-tests
      # entry; that list is the audit surface, and every entry on it is
      # accounted for in docs/TEST-OVERRIDES.md.
      (rc_ok, ok_out) = machine.execute("test -f /testok")
      (rc_skip, skip_out) = machine.execute("test -f /skipped")
      if rc_ok != 0 and rc_skip != 0:
          print("=== /testok and /skipped both missing, dumping journal for diagnostics ===")
          (rc_j, j) = machine.execute("journalctl --no-pager -b 2>&1 | tail -400")
          print(j)
          print("=== systemctl list-units --failed ===")
          (rc_f, f) = machine.execute("systemctl list-units --failed 2>&1")
          print(f)

      if ${if expectedSkip then "True" else "False"}:
          # Opted in to skipping: either marker is accepted, because such a
          # test may also run to completion once its prerequisite appears.
          machine.succeed("test -f /testok -o -f /skipped")
          if rc_ok == 0:
              print("NOTE: expectedSkip is set but the test reached /testok. "
                    "Drop expectedSkip from this test's integration-tests entry.")
      else:
          if rc_ok != 0 and rc_skip == 0:
              print("=== test wrote /skipped, so nothing was verified ===")
              (rc_s, s) = machine.execute("cat /skipped 2>&1")
              print(s)
              raise Exception(
                  "test skipped itself (/skipped) but is not marked expectedSkip; "
                  "a skip is not a pass, see docs/TEST-OVERRIDES.md"
              )
          machine.succeed("test -f /testok")
    '';
  }
