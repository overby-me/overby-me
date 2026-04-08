{
  devShells.rust-systemd = pkgs: {
    packages = with pkgs; [
      just
      (rust-bin.stable.latest.default.override {
        extensions = ["rust-src"];
        targets = ["x86_64-unknown-linux-gnu" "x86_64-unknown-uefi"];
      })
    ];
  };

  packages = {
    rust-systemd = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
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

        cargoLock.lockFile = ./Cargo.lock;

        doCheck = false;

        meta = {
          description = "A service manager that is able to run \"traditional\" systemd services, written in rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/systemd";
          license = lib.licenses.mit;
          maintainers = with lib.maintainers; [noverby];
          mainProgram = "systemd";
        };
      };

    rust-systemd-drowse = {
      drowse,
      lib,
    }:
      drowse.crate2nix {
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

        #dynamicCargoDeps = false;

        select = ''
          project:
          let
            pkgs = import <nixpkgs> {};
            members = builtins.attrValues (builtins.mapAttrs (_: m: m.build) project.workspaceMembers);
          in
          pkgs.runCommand "rust-systemd" {} '''
            mkdir -p $out/bin
            for pkg in ''${pkgs.lib.concatMapStringsSep " " toString members}; do
              for bin in $pkg/bin/*; do
                cp -a "$bin" "$out/bin/"
              done
            done
          '''
        '';

        doCheck = false;

        meta = {
          description = "A service manager that is able to run \"traditional\" systemd services, written in rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/systemd";
          license = lib.licenses.mit;
          maintainers = with lib.maintainers; [noverby];
          mainProgram = "systemd";
        };
      };

    rust-systemd-systemd = {
      runCommand,
      makeBinaryWrapper,
      rust-systemd,
      kbd,
      kmod,
      util-linuxMinimal,
      systemd,
    }:
      runCommand "rust-systemd-systemd-${rust-systemd.version}" {
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
          withMachined = true;
          withNetworkd = true;
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
      } ''
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

                # Overwrite with rust-systemd binaries (takes precedence)
                for bin in ${rust-systemd}/bin/*; do
                  name=$(basename "$bin")
                  cp -a "$bin" "$out/bin/$name"
                done

                # Provide sbin as a symlink to bin (matching systemd layout)
                if [ ! -e "$out/sbin" ]; then
                  ln -s bin "$out/sbin"
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
                    cp -a "$bin" "$out/lib/systemd/$name"
                  fi
                done

                # Install rust-only binaries that don't exist in the C systemd package.
                # These are new binaries implemented in rust-systemd without a C counterpart.
                for name in systemd-bsod systemd-journal-gatewayd systemd-journal-remote systemd-journal-upload; do
                  if [ -e "${rust-systemd}/bin/$name" ] && [ ! -e "$out/lib/systemd/$name" ]; then
                    cp -a "${rust-systemd}/bin/$name" "$out/lib/systemd/$name"
                  fi
                done

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

        [Install]
        WantedBy=multi-user.target
        UPLOAD_SERVICE

                # Install test binaries at paths expected by upstream integration tests.
                mkdir -p $out/lib/systemd/tests/unit-tests/manual
                for name in test-journal-append; do
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
    # Upstream systemd integration test names (without TEST- prefix).
    # Each corresponds to test/units/TEST-{name}.sh in the systemd source.
    # Run with: nix build .#checks.x86_64-linux.rust-systemd-test-{name}
    testFiles =
      builtins.filter
      (f: builtins.match ".*\.nix" f != null)
      (builtins.attrNames (builtins.readDir ./integration-tests));
    # Each test gets its check name from the filename (e.g. "04-journal-bsod.nix" -> "04-journal-bsod")
    # and the upstream test script name from t.name (e.g. "04-JOURNAL").
    tests = map (f:
      (import (./integration-tests + "/${f}"))
      // {
        _checkName = builtins.replaceStrings [".nix"] [""] f;
      })
    testFiles;
  in
    builtins.listToAttrs ((map (t: {
          name = "rust-systemd-test-${t._checkName}";
          value = pkgs:
            import ./testsuite.nix {
              inherit pkgs;
              inherit (t) name;
              patchScript = t.patchScript or "";
              extraPackages = (t.extraPackages or (_: [])) pkgs;
              testEnv = t.testEnv or {};
              testTimeout = t.testTimeout or 1800;
            };
        })
        tests)
      ++ (map (t: {
          name = "c-systemd-test-${t._checkName}";
          value = pkgs:
            import ./testsuite.nix {
              inherit pkgs;
              inherit (t) name;
              patchScript = t.patchScript or "";
              extraPackages = (t.extraPackages or (_: [])) pkgs;
              testEnv = t.testEnv or {};
              testTimeout = t.testTimeout or 1800;
              useUpstreamSystemd = true;
            };
        })
        tests));
}
