{
  packages = {
    rust-pipewire = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-pipewire";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        postInstall = ''
          # Multicall binary: every PipeWire tool/daemon name is a symlink
          # to rust-pipewire. argv[0] selects the dispatcher.
          for tool in \
            pipewire pipewire-pulse pipewire-aes67 pipewire-avb pipewire-vulkan \
            pw-cli pw-mon pw-dump pw-link pw-metadata pw-loopback pw-config \
            pw-cat pw-play pw-record pw-dot pw-top pw-profiler pw-reserve \
            pw-container pw-mididump pw-midiplay pw-midirecord pw-midi2play \
            pw-midi2record pw-sysex pw-dsdplay pw-encplay pw-v4l2 \
            spa-json-dump spa-inspect spa-monitor spa-acp-tool spa-resample; do
            ln -s $out/bin/rust-pipewire $out/bin/$tool
          done
        '';

        meta = {
          description = "PipeWire-compatible multimedia graph daemon and tools written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/pipewire";
          license = lib.licenses.mit;
          mainProgram = "pw-cli";
        };
      };

    rust-pipewire-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-pipewire-dev";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        buildType = "debug";

        postInstall = ''
          for tool in \
            pipewire pipewire-pulse pipewire-aes67 pipewire-avb pipewire-vulkan \
            pw-cli pw-mon pw-dump pw-link pw-metadata pw-loopback pw-config \
            pw-cat pw-play pw-record pw-dot pw-top pw-profiler pw-reserve \
            pw-container pw-mididump pw-midiplay pw-midirecord pw-midi2play \
            pw-midi2record pw-sysex pw-dsdplay pw-encplay pw-v4l2 \
            spa-json-dump spa-inspect spa-monitor spa-acp-tool spa-resample; do
            ln -s $out/bin/rust-pipewire $out/bin/$tool
          done
        '';

        meta = {
          description = "PipeWire-compatible multimedia graph daemon and tools written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/pipewire";
          license = lib.licenses.mit;
          mainProgram = "pw-cli";
        };
      };
  };

  checks = let
    testDefs = [
      {
        tool = "pw-cli";
        name = "help";
      }
      {
        tool = "spa-json-dump";
        name = "help";
      }
      {
        tool = "spa-json-dump";
        name = "basic";
      }
      {
        tool = "spa-json-dump";
        name = "spa-format";
      }
      {
        tool = "spa-json-dump";
        name = "indent-4";
      }
      {
        tool = "spa-json-dump";
        name = "simplified";
      }
      {
        tool = "spa-json-dump";
        name = "comments";
      }
      {
        tool = "spa-json-dump";
        name = "numbers";
      }
      {
        tool = "spa-json-dump";
        name = "strings";
      }
      {
        tool = "spa-json-dump";
        name = "nested";
      }
      {
        tool = "spa-json-dump";
        name = "conf-minimal";
      }
      {
        tool = "spa-json-dump";
        name = "conf-client";
      }
      {
        tool = "spa-json-dump";
        name = "conf-jack";
      }
      {
        tool = "spa-json-dump";
        name = "conf-aes67";
      }

      # --- per-tool --version byte-identical parity ---
      { tool = "pw-cli"; name = "version"; }
      { tool = "pw-mon"; name = "version"; }
      { tool = "pw-link"; name = "version"; }
      { tool = "pw-metadata"; name = "version"; }
      { tool = "pw-config"; name = "version"; }
      { tool = "pw-dump"; name = "version"; }
      { tool = "pw-dot"; name = "version"; }
      { tool = "pw-mididump"; name = "version"; }
      { tool = "pw-profiler"; name = "version"; }
      { tool = "pw-top"; name = "version"; }
      { tool = "pipewire"; name = "version"; }
      { tool = "pipewire-pulse"; name = "version"; }

      # --- per-tool --help byte-identical parity ---
      {
        tool = "pw-config";
        name = "help";
      }
      {
        tool = "pw-dot";
        name = "help";
      }
      {
        tool = "pw-dump";
        name = "help";
      }
      {
        tool = "pw-link";
        name = "help";
      }
      {
        tool = "pw-metadata";
        name = "help";
      }
      {
        tool = "pw-mididump";
        name = "help";
      }
      {
        tool = "pw-mon";
        name = "help";
      }
      {
        tool = "pw-profiler";
        name = "help";
      }
      {
        tool = "pw-top";
        name = "help";
      }
      {
        tool = "pipewire";
        name = "help";
      }
      {
        tool = "pipewire-pulse";
        name = "help";
      }

      # --- pw-mididump SMF parsing tests ---
      {
        tool = "pw-mididump";
        name = "basic";
      }
      {
        tool = "pw-mididump";
        name = "controllers";
      }
      {
        tool = "pw-mididump";
        name = "tempo-meta";
      }
      {
        tool = "pw-mididump";
        name = "text-meta";
      }
      {
        tool = "pw-mididump";
        name = "running-status";
      }

      # --- pw-config offline tests ---
      {
        tool = "pw-config";
        name = "paths-single";
      }
      {
        tool = "pw-config";
        name = "paths-with-overrides";
      }
      {
        tool = "pw-config";
        name = "paths-no-newline";
      }
      {
        tool = "pw-config";
        name = "paths-custom-name";
      }
    ];

    # POD comparison tests — built separately because they compile a
    # libspa-linked helper instead of running an existing C tool.
    podTests = [
      {name = "encode-cases";}
    ];

    # Daemon-interop tests — spawn a real C pipewire daemon and probe it
    # with our protocol-native client.
    protoTests = [
      {name = "hello-info";}
    ];

    # Rich-daemon tests — same as daemonTests but with a null-audio-sink
    # Node pre-loaded so Node/Port code paths can be exercised.
    richDaemonTests = [
      {tool = "pw-cli"; name = "ls-node-rich";}
      {tool = "pw-cli"; name = "ls-port-rich";}
      {tool = "pw-cli"; name = "info-node-rich";}
      {tool = "pw-cli"; name = "info-all-rich";}
      {tool = "pw-cli"; name = "ls-substring-rich";}
      {tool = "pw-cli"; name = "info-by-name-rich";}
      {tool = "pw-link"; name = "input-rich";}
      {tool = "pw-link"; name = "output-rich";}
      {tool = "pw-link"; name = "input-id-rich";}
      {tool = "pw-link"; name = "input-verbose-rich";}
      {tool = "pw-metadata"; name = "list-rich";}
      {tool = "pw-metadata"; name = "list-byname";}
    ];

    # Daemon-comparison tests — spawn a real C pipewire daemon and run both
    # the C tool and the Rust tool against it, then diff. Each entry is
    # `tools/<tool>/<name>.sh`.
    daemonTests = [
      {tool = "pw-cli"; name = "ls-core";}
      {tool = "pw-cli"; name = "ls-module";}
      {tool = "pw-cli"; name = "ls-factory";}
      {tool = "pw-cli"; name = "ls-securitycontext";}
      {tool = "pw-cli"; name = "ls-metadata";}
      {tool = "pw-cli"; name = "ls-empty-node";}
      {tool = "pw-cli"; name = "ls-empty-link";}
      {tool = "pw-cli"; name = "ls-empty-port";}
      {tool = "pw-cli"; name = "ls-empty-device";}
      {tool = "pw-cli"; name = "ls-all-normalized";}
      {tool = "pw-cli"; name = "ls-substring";}
      {tool = "pw-cli"; name = "info-core";}
      {tool = "pw-cli"; name = "info-module-1";}
      {tool = "pw-cli"; name = "info-module-3";}
      {tool = "pw-cli"; name = "info-module-5";}
      {tool = "pw-cli"; name = "info-factory";}
      {tool = "pw-cli"; name = "info-all";}
      {tool = "pw-cli"; name = "info-bad-id";}
      {tool = "pw-cli"; name = "info-securitycontext";}
      {tool = "pw-cli"; name = "info-by-name";}
      {tool = "pw-cli"; name = "help-cmd";}
      {tool = "pw-cli"; name = "list-vars";}
      {tool = "pw-cli"; name = "list-remotes";}
      {tool = "pw-cli"; name = "quit";}
      {tool = "pw-cli"; name = "usage-load-module";}
      {tool = "pw-cli"; name = "usage-unload-module";}
      {tool = "pw-cli"; name = "usage-create-device";}
      {tool = "pw-cli"; name = "usage-create-node";}
      {tool = "pw-cli"; name = "usage-destroy";}
      {tool = "pw-cli"; name = "usage-enum-params";}
      {tool = "pw-cli"; name = "usage-set-param";}
      {tool = "pw-cli"; name = "usage-permissions";}
      {tool = "pw-cli"; name = "usage-send-command";}
      {tool = "pw-cli"; name = "usage-get-permissions";}
      {tool = "pw-cli"; name = "usage-create-link";}
      {tool = "pw-cli"; name = "usage-export-node";}
      {tool = "pw-link"; name = "empty-input";}
      {tool = "pw-link"; name = "empty-output";}
      {tool = "pw-link"; name = "empty-links";}
      {tool = "pw-metadata"; name = "list-empty";}
      {tool = "pw-dump"; name = "structural";}
      {tool = "pw-dump"; name = "structural-indent4";}
      {tool = "pw-dump"; name = "structural-indent0";}
    ];
  in
    builtins.listToAttrs (
      (map (t: {
          name = "rust-pipewire-test-${t.tool}-${t.name}";
          value = pkgs:
            import ./testsuite.nix {
              inherit pkgs;
              inherit (t) tool name;
            };
        })
        testDefs)
      ++ (map (t: {
          name = "rust-pipewire-pod-test-${t.name}";
          value = pkgs:
            import ./pod-testsuite.nix {
              inherit pkgs;
              inherit (t) name;
            };
        })
        podTests)
      ++ (map (t: {
          name = "rust-pipewire-proto-test-${t.name}";
          value = pkgs:
            import ./proto-testsuite.nix {
              inherit pkgs;
              inherit (t) name;
            };
        })
        protoTests)
      ++ (map (t: {
          name = "rust-pipewire-daemon-test-${t.tool}-${t.name}";
          value = pkgs:
            import ./daemon-testsuite.nix {
              inherit pkgs;
              inherit (t) tool name;
            };
        })
        daemonTests)
      ++ (map (t: {
          name = "rust-pipewire-rich-daemon-test-${t.tool}-${t.name}";
          value = pkgs:
            import ./rich-daemon-testsuite.nix {
              inherit pkgs;
              inherit (t) tool name;
            };
        })
        richDaemonTests)
    );
}
