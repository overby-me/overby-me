{lib, ...}: let
  # Add systemd's pam_systemd.so to linux-pam's securedir so the manager's
  # dlopened libpam (see exec_helper.rs PAMName= support) can resolve the bare
  # `pam_systemd.so` the 35-LOGIN PAM stack references. NixOS pam.d files
  # normally use absolute module paths, so the stock securedir omits it.
  mkPamWithSystemd = {
    pam,
    systemd,
  }:
    pam.overrideAttrs (old: {
      postInstall =
        (old.postInstall or "")
        + ''
          ln -sf ${systemd}/lib/security/pam_systemd.so $out/lib/security/pam_systemd.so
        '';
    });
in {
  devShells.rust-systemd = pkgs: {
    packages = with pkgs; [
      just
      (rust-bin.stable.latest.default.override {
        extensions = ["rust-src"];
        targets = [
          "x86_64-unknown-linux-gnu"
          "x86_64-unknown-uefi"
        ];
      })
    ];
  };

  packages = {
    rust-systemd = {lib, pam, systemd, acl, shadow, ...}:
      lib.buildCargoProject {
        pname = "rust-systemd";
        version = "unstable";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./crates
          ];
        };

        index = ../../nix/lib/cargo/index;

        features = ["dbus_support"];

        # libsystemd dlopens libpam at runtime for PAMName= session setup; bake
        # the absolute library path in via PAM_LIB (see exec_helper.rs).
        crateOverrides = {
          libsystemd = {
            PAM_LIB = "${mkPamWithSystemd {inherit pam systemd;}}/lib/libpam.so.0";
          };
          # systemd-tmpfiles shells out to setfacl for POSIX ACL rules; bake its
          # absolute path (ACL_SETFACL) so it resolves from the boot-time
          # systemd-tmpfiles-setup service's minimal $PATH.
          systemd-tmpfiles = {
            ACL_LIB = "${acl.out}/lib/libacl.so.1";
          };
          # systemd-sysusers shells out to useradd/groupadd/chage; bake shadow's
          # bin dir (SHADOW_BIN) so they resolve from a minimal service $PATH.
          systemd-sysusers = {
            SHADOW_BIN = "${shadow}/bin";
          };
        };

        meta = {
          description = "A service manager that is able to run \"traditional\" systemd services, written in rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/systemd";
          license = lib.licenses.mit;
          maintainers = with lib.maintainers; [overby-me];
          mainProgram = "systemd";
          platforms = lib.platforms.linux;
        };
      };

    # Fast-iteration development build used as the integration-test manager:
    # debug profile, default LLVM codegen, wild linker.  ~15s hot rebuilds
    # after a libsystemd change (vs ~49s release).  More importantly, the debug
    # profile keeps integer-overflow checks and debug_assert!s compiled in, so
    # latent manager bugs surface as visible PID 1 panics instead of the silent
    # wraparound a release build would produce.  Behavior otherwise matches the
    # release build; only optimization and these runtime checks differ.
    rust-systemd-dev = {lib, pam, systemd, acl, shadow, ...}:
      lib.buildCargoProject {
        pname = "rust-systemd-dev";
        version = "unstable";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./crates
          ];
        };

        index = ../../nix/lib/cargo/index;

        features = ["dbus_support"];
        release = false;

        # libsystemd dlopens libpam at runtime for PAMName= session setup; bake
        # the absolute library path in via PAM_LIB (see exec_helper.rs).
        crateOverrides = {
          libsystemd = {
            PAM_LIB = "${mkPamWithSystemd {inherit pam systemd;}}/lib/libpam.so.0";
          };
          # systemd-tmpfiles shells out to setfacl for POSIX ACL rules; bake its
          # absolute path (ACL_SETFACL) so it resolves from the boot-time
          # systemd-tmpfiles-setup service's minimal $PATH.
          systemd-tmpfiles = {
            ACL_LIB = "${acl.out}/lib/libacl.so.1";
          };
          # systemd-sysusers shells out to useradd/groupadd/chage; bake shadow's
          # bin dir (SHADOW_BIN) so they resolve from a minimal service $PATH.
          systemd-sysusers = {
            SHADOW_BIN = "${shadow}/bin";
          };
        };

        meta = {
          description = "rust-systemd built for fast development iteration (debug + wild)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/systemd";
          license = lib.licenses.mit;
          mainProgram = "systemd";
          platforms = lib.platforms.linux;
        };
      };

    # rust-systemd-drowse = {
    #   drowse,
    #   lib,
    # }:
    # # crate2nix constructs its own derivation and drops the `meta` we pass in,
    # # so re-apply meta.platforms via overrideAttrs to ensure meta.available is
    # # false on non-Linux (otherwise nix flake check forces it on darwin and the
    # # crate2nix builtins.outputOf usage fails).
    #   (drowse.crate2nix {
    #     pname = "rust-systemd";
    #     version = "unstable";

    #     src = lib.fileset.toSource {
    #       root = ./.;
    #       fileset = lib.fileset.unions [
    #         ./Cargo.toml
    #         ./Cargo.lock
    #         ./crates
    #       ];
    #     };

    #     #dynamicCargoDeps = false;

    #     select = ''
    #       project:
    #       let
    #         pkgs = import <nixpkgs> {};
    #         members = lib.attrValues (lib.mapAttrs (_: m: m.build) project.workspaceMembers);
    #       in
    #       pkgs.runCommand "rust-systemd" {} '''
    #         mkdir -p $out/bin
    #         for pkg in ''${pkgs.lib.concatMapStringsSep " " toString members}; do
    #           for bin in $pkg/bin/*; do
    #             cp -a "$bin" "$out/bin/"
    #           done
    #         done
    #       '''
    #     '';

    #     doCheck = false;

    #     meta = {
    #       description = "A service manager that is able to run \"traditional\" systemd services, written in rust";
    #       homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/systemd";
    #       license = lib.licenses.mit;
    #       maintainers = with lib.maintainers; [overby-me];
    #       mainProgram = "systemd";
    #       platforms = lib.platforms.linux;
    #     };
    #   }).overrideAttrs
    #   (old: {
    #     meta =
    #       (old.meta or {})
    #       // {
    #         platforms = lib.platforms.linux;
    #       };
    #   });

    rust-systemd-systemd = {
      runCommand,
      makeBinaryWrapper,
      rust-systemd-dev,
      kbd,
      kmod,
      util-linuxMinimal,
      systemd,
      # `rust-systemd` (the release build) is still passed by the package
      # framework; absorb it with `...` since we build from the dev variant.
      ...
    }: let
      # Build the integration-test manager from the fast debug+wild dev build
      # (see rust-systemd-dev above): quick rebuilds plus debug_assert!/overflow
      # checks that surface manager bugs a release build would hide.
      rust-systemd = rust-systemd-dev;
    in
      runCommand "rust-systemd-systemd-${rust-systemd.version}"
      {
        nativeBuildInputs = [makeBinaryWrapper];

        passthru = {
          inherit kbd kmod;
          util-linux = util-linuxMinimal;
          interfaceVersion = 2;
          withBootloader = false;
          withCryptsetup = false;
          withEfi = false;
          withFido2 = false;
          withHostnamed = true;
          withImportd = false;
          withKmod = false;
          withLocaled = true;
          withLogind = true;
          withMachined = true;
          withNetworkd = true;
          withNspawn = true;
          withHomed = true;
          withPortabled = true;
          withSysupdate = false;
          withTimedated = true;
          withTpm2Tss = false;
          withTpm2Units = false;
          withUtmp = false;
        };

        meta =
          rust-systemd.meta
          // {
            description = "rust-systemd packaged as a systemd drop-in replacement for NixOS";
          };
      }
      ''
                mkdir -p $out

                # Copy data/config files from systemd that NixOS modules expect
                cp -r ${systemd}/example $out/example
                cp -r ${systemd}/lib $out/lib
                cp -r ${systemd}/etc $out/etc 2>/dev/null || true
                cp -r ${systemd}/share $out/share 2>/dev/null || true

                # Make copied files writable so we can overwrite them
                chmod -R u+w $out

                # Start with all systemd binaries
                mkdir -p $out/bin
                for bin in ${systemd}/bin/*; do
                  name=$(basename "$bin")
                  cp -a "$bin" "$out/bin/$name"
                done

                # Make copied binaries writable so rust-systemd can overwrite them
                chmod -R u+w $out/bin

                # Overwrite with rust-systemd binaries (takes precedence).
                # Dereference (-L): the cargo abstraction ships these as
                # symlinks into per-crate store paths (udevadm-0.1.0, …), and
                # the stage-1 initrd's self-contained `extra-utils` refuses to
                # reference those derivations. Copying the real binaries (which
                # link only glibc) keeps this package reference-clean.
                for bin in ${rust-systemd}/bin/*; do
                  name=$(basename "$bin")
                  cp -aL "$bin" "$out/bin/$name"
                done

                # Provide sbin as a symlink to bin (matching systemd layout)
                if [ ! -e "$out/sbin" ]; then
                  ln -s bin "$out/sbin"
                fi

                # run0 is a multi-call alias of systemd-run: crates/run detects
                # argv[0] == "run0" and elevates privileges (runs COMMAND as the
                # target user in a transient unit).  Provide it as a symlink.
                if [ -e "$out/bin/systemd-run" ]; then
                  ln -sf systemd-run "$out/bin/run0"
                fi

                # Replace the systemd init binary with a wrapper that execs rust-systemd,
                # so NixOS actually boots with rust-systemd as PID 1 instead of systemd.
                # NixOS uses $out/lib/systemd/systemd as the init binary (stage-2).
                # We can't symlink because rust-systemd's main() dispatches on argv[0]
                # ending with "rust-systemd" or "systemd", so we need a wrapper script.
                rm -f $out/lib/systemd/systemd
                makeBinaryWrapper ${rust-systemd}/bin/systemd $out/lib/systemd/systemd \
                  --argv0 rust-systemd

                # Replace lib/systemd/* helper binaries with rust-systemd equivalents.
                # Many service units use ExecStart=$out/lib/systemd/systemd-<foo> rather
                # than $out/bin/systemd-<foo>, so we need to overwrite those too.
                for bin in ${rust-systemd}/bin/*; do
                  name=$(basename "$bin")
                  if [ -e "$out/lib/systemd/$name" ]; then
                    rm -f "$out/lib/systemd/$name"
                    cp -aL "$bin" "$out/lib/systemd/$name"
                  fi
                done

                # Install rust-only binaries that don't exist in the C systemd package.
                # These are new binaries implemented in rust-systemd without a C counterpart.
                for name in systemd-bsod systemd-journal-gatewayd systemd-journal-remote systemd-journal-upload systemd-battery-check systemd-report; do
                  if [ -e "${rust-systemd}/bin/$name" ] && [ ! -e "$out/lib/systemd/$name" ]; then
                    cp -aL "${rust-systemd}/bin/$name" "$out/lib/systemd/$name"
                  fi
                done

                # Install our systemd-fstab-generator into the standard
                # generator path.  Overrides the C version (if present)
                # since NixOS' systemd package may not ship it, and
                # TEST-81-GENERATORS.fstab-generator expects the binary
                # at the canonical location.
                if [ -e "${rust-systemd}/bin/systemd-fstab-generator" ]; then
                  mkdir -p "$out/lib/systemd/system-generators"
                  cp -aL "${rust-systemd}/bin/systemd-fstab-generator" \
                    "$out/lib/systemd/system-generators/systemd-fstab-generator"
                fi


                # Install systemd-bsod.service — C systemd doesn't build it without qrencode,
                # but our Rust implementation doesn't need qrencode.
                mkdir -p "$out/lib/systemd/system"
                cat > "$out/lib/systemd/system/systemd-bsod.service" <<BSOD_UNIT
        [Unit]
        Description=Display Boot-Time Emergency Messages In Full Screen
        ConditionVirtualization=no
        DefaultDependencies=no
        Before=shutdown.target
        Conflicts=shutdown.target

        [Service]
        RemainAfterExit=yes
        ExecStart=$out/lib/systemd/systemd-bsod --continuous
        BSOD_UNIT

                # Install systemd-journal-gatewayd service and socket units
                cat > "$out/lib/systemd/system/systemd-journal-gatewayd.service" <<GATEWAYD_SERVICE
        [Unit]
        Description=Journal Gateway Service
        Requires=systemd-journal-gatewayd.socket

        [Service]
        ExecStart=$out/lib/systemd/systemd-journal-gatewayd
        SupplementaryGroups=systemd-journal
        LimitNOFILE=524288

        [Install]
        Also=systemd-journal-gatewayd.socket
        GATEWAYD_SERVICE
                cat > "$out/lib/systemd/system/systemd-journal-gatewayd.socket" <<GATEWAYD_SOCKET
        [Unit]
        Description=Journal Gateway Service Socket

        [Socket]
        ListenStream=19531

        [Install]
        WantedBy=sockets.target
        GATEWAYD_SOCKET

                # Install systemd-journal-remote service and socket units
                cat > "$out/lib/systemd/system/systemd-journal-remote.service" <<REMOTE_SERVICE
        [Unit]
        Description=Journal Remote Sink Service
        Requires=systemd-journal-remote.socket

        [Service]
        ExecStart=$out/lib/systemd/systemd-journal-remote --listen-https=-3 --output=/var/log/journal/remote/

        [Install]
        Also=systemd-journal-remote.socket
        REMOTE_SERVICE
                cat > "$out/lib/systemd/system/systemd-journal-remote.socket" <<REMOTE_SOCKET
        [Unit]
        Description=Journal Remote Sink Socket

        [Socket]
        ListenStream=19532
        REMOTE_SOCKET

                # Install systemd-journal-upload service unit
                cat > "$out/lib/systemd/system/systemd-journal-upload.service" <<UPLOAD_SERVICE
        [Unit]
        Description=Journal Remote Upload Service
        Wants=network-online.target
        After=network-online.target

        [Service]
        ExecStart=$out/lib/systemd/systemd-journal-upload --save-state
        SupplementaryGroups=systemd-journal
        StateDirectory=systemd/journal-upload
        UPLOAD_SERVICE

                # Install the user-manager units.  The C package ships these under
                # example/systemd/system with ExecStart= pointing at the C binary, so
                # linking those in would start the C manager under rust PID 1.  Write
                # our own into BOTH lib/ and example/ (the extraUnits search in
                # testsuite.nix looks at example/ first, so the copy there has to be
                # ours too) pointing at the rust manager.
                #
                # Type=notify-reload matches upstream and works because
                # run_user_manager() sends READY=1 once its control socket is bound.
                for dir in "$out/lib/systemd/system" "$out/example/systemd/system"; do
                  mkdir -p "$dir"
                  rm -f "$dir/user@.service" "$dir/user-runtime-dir@.service"

                  cat > "$dir/user@.service" <<USER_SERVICE
        [Unit]
        Description=User Manager for UID %i
        BindsTo=user-runtime-dir@%i.service
        After=systemd-logind.service user-runtime-dir@%i.service
        IgnoreOnIsolate=yes

        [Service]
        User=%i
        Type=notify-reload
        # Upstream gets XDG_RUNTIME_DIR from PAMName=systemd-user; set it
        # explicitly so the manager's %t, %S and %C resolve per-user.
        Environment=XDG_RUNTIME_DIR=/run/user/%i
        ExecStart=$out/lib/systemd/systemd --user
        Slice=user-%i.slice
        # Without this the manager runs as the user but its cgroup stays
        # root-owned, so every service it tries to start dies with EACCES on
        # cgroup creation. Matches upstream units/user@.service.in.
        Delegate=pids memory cpu
        KillMode=mixed
        TasksMax=infinity
        TimeoutStopSec=120s
        KeyringMode=inherit
        USER_SERVICE

                  # rust-systemd has no systemd-user-runtime-dir binary; the
                  # directory is all the manager needs for XDG_RUNTIME_DIR.
                  cat > "$dir/user-runtime-dir@.service" <<RUNTIME_DIR_SERVICE
        [Unit]
        Description=User Runtime Directory /run/user/%i
        After=systemd-logind.service
        IgnoreOnIsolate=yes

        [Service]
        Type=oneshot
        RemainAfterExit=yes
        ExecStart=/bin/sh -c 'mkdir -p /run/user/%i && chmod 0700 /run/user/%i && chown %i /run/user/%i'
        ExecStop=/bin/sh -c 'rm -rf /run/user/%i'
        RUNTIME_DIR_SERVICE
                done

                # Install test binaries at paths expected by upstream integration tests.
                mkdir -p $out/lib/systemd/tests/unit-tests/manual
                for name in test-journal-append test-sleep test-thp; do
                  if [ -e "${rust-systemd}/bin/$name" ]; then
                    cp -a "${rust-systemd}/bin/$name" "$out/lib/systemd/tests/unit-tests/manual/$name"
                  fi
                done

                # Replace all references to the real systemd store path with
                # the rust-systemd-systemd output path so NixOS module substitutions work.
                #
                # NOTE: Only text files are patched. ELF binaries (e.g. udevadm) have
                # the original systemd store path compiled into their RPATH and default
                # config/rules directories. Binary string substitution is NOT safe here
                # because the store paths are different lengths (the original systemd
                # path like "...-systemd-258.3" is shorter than our overlay path like
                # "...-rust-systemd-systemd-unstable"), so replacing would corrupt the
                # binary layout. This means udevd will still read its built-in rules
                # from the original systemd package — a cosmetic issue until udevd is
                # reimplemented in Rust.
                find $out -type f | while read -r f; do
                  if file "$f" | grep -q text; then
                    substituteInPlace "$f" \
                      --replace-quiet "${systemd}" "$out"
                  fi
                done

                # Fix broken symlinks that pointed within the systemd package
                find $out -type l | while read -r link; do
                  target=$(readlink "$link")
                  if [[ "$target" == ${systemd}* ]]; then
                    newtarget="$out''${target#${systemd}}"
                    ln -sf "$newtarget" "$link"
                  fi
                done
      '';
  };

  checks = let
    # The upstream systemd version the port is audited and its oracles build
    # against. `pkgs.systemd` (the differential oracles and every c-systemd-* check
    # use it) must match this, or the comparison runs against a version the port was
    # never checked against. The `rust-systemd-upstream-pin` check below turns a
    # silent drift here into a failing check (porting rule 8). Bump only after
    # re-auditing against the new version; docs/ARCHITECTURE, docs/ROADMAP and
    # docs/TEST-OVERRIDES cite the same number in prose.
    expectedSystemdVersion = "260.2";

    # Upstream systemd integration test names (without TEST- prefix).
    # Each corresponds to test/units/TEST-{name}.sh in the systemd source.
    # Run with: nix build .#checks.x86_64-linux.rust-systemd-test-{name}
    testFiles = lib.filter (f: lib.match ".*\.nix" f != null) (
      lib.attrNames (lib.readDir ./integration-tests)
    );
    # Each test gets its check name from the filename (e.g. "04-journal-bsod.nix" -> "04-journal-bsod")
    # and the upstream test script name from t.name (e.g. "04-JOURNAL").
    tests =
      map (
        f:
          (import (./integration-tests + "/${f}"))
          // {
            _checkName = lib.replaceStrings [".nix"] [""] f;
          }
      )
      testFiles;
    # Every key an integration-tests entry may set, with its default.  Kept in
    # one place so the rust-systemd and c-systemd variants cannot drift apart.
    testArgs = t: pkgs: {
      inherit pkgs;
      inherit (t) name;
      patchScript = t.patchScript or "";
      extraPackages = (t.extraPackages or (_: [])) pkgs;
      extraUnits = t.extraUnits or [];
      testEnv = t.testEnv or {};
      testTimeout = t.testTimeout or 1800;
      enableTpm = t.enableTpm or false;
      allowReboot = t.allowReboot or false;
      useBootLoader = t.useBootLoader or false;
      expectedSkip = t.expectedSkip or false;
      jobGraph = t.jobGraph or false;
      initScope = t.initScope or false;
    };
  in
    lib.listToAttrs (
      (map (t: {
          name = "rust-systemd-test-${t._checkName}";
          value = pkgs: import ./testsuite.nix (testArgs t pkgs);
        })
        tests)
      ++ (map (t: {
          name = "c-systemd-test-${t._checkName}";
          value = pkgs:
            import ./testsuite.nix (testArgs t pkgs // {useUpstreamSystemd = true;});
        })
        tests)
      ++ [
        {
          # Upstream-pin drift guard (porting rule 8): fail if the nixpkgs systemd
          # the oracles build against has moved off the version the port was
          # audited at, so drift surfaces as a failing check instead of silent
          # parity decay. Cheap (a version-string compare); no VM.
          name = "rust-systemd-upstream-pin";
          value = pkgs:
            pkgs.runCommand "rust-systemd-upstream-pin" {
              expected = expectedSystemdVersion;
              actual = pkgs.systemd.version;
            } ''
              if [ "$expected" != "$actual" ]; then
                echo "systemd upstream pin drift: the port targets $expected but nixpkgs provides $actual." >&2
                echo "Re-audit rust-systemd against $actual, then bump expectedSystemdVersion in default.nix (and the docs that cite it)." >&2
                exit 1
              fi
              echo "systemd upstream pin OK: $expected" > "$out"
            '';
        }
      ]
    );
}
