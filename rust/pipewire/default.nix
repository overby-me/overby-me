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
        name = "bad-file";
      }
      {
        tool = "spa-json-dump";
        name = "bad-flag";
      }
      {
        tool = "spa-json-dump";
        name = "invalid-V";
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
      { tool = "pw-cat"; name = "help"; }
      { tool = "pw-play"; name = "help"; }
      { tool = "pw-record"; name = "help"; }
      { tool = "pw-midiplay"; name = "help"; }
      { tool = "pw-midirecord"; name = "help"; }
      { tool = "pw-midi2play"; name = "help"; }
      { tool = "pw-midi2record"; name = "help"; }
      { tool = "pw-sysex"; name = "help"; }
      { tool = "pw-dsdplay"; name = "help"; }
      { tool = "pw-encplay"; name = "help"; }
      { tool = "pw-cat"; name = "version"; }
      { tool = "pw-play"; name = "version"; }
      { tool = "pw-record"; name = "version"; }
      { tool = "pw-midiplay"; name = "version"; }
      { tool = "pw-midirecord"; name = "version"; }
      { tool = "pw-midi2play"; name = "version"; }
      { tool = "pw-midi2record"; name = "version"; }
      { tool = "pw-sysex"; name = "version"; }
      { tool = "pw-dsdplay"; name = "version"; }
      { tool = "pw-encplay"; name = "version"; }
      { tool = "pw-cat"; name = "no-args"; }
      { tool = "pw-play"; name = "no-args"; }
      { tool = "pw-record"; name = "no-args"; }
      { tool = "pw-midiplay"; name = "no-args"; }
      { tool = "pw-midirecord"; name = "no-args"; }
      { tool = "pw-midi2play"; name = "no-args"; }
      { tool = "pw-midi2record"; name = "no-args"; }
      { tool = "pw-sysex"; name = "no-args"; }
      { tool = "pw-dsdplay"; name = "no-args"; }
      { tool = "pw-encplay"; name = "no-args"; }
      { tool = "pw-cat"; name = "help-short"; }
      { tool = "pw-play"; name = "help-short"; }
      { tool = "pw-record"; name = "help-short"; }
      { tool = "pw-midiplay"; name = "help-short"; }
      { tool = "pw-midirecord"; name = "help-short"; }
      { tool = "pw-midi2play"; name = "help-short"; }
      { tool = "pw-midi2record"; name = "help-short"; }
      { tool = "pw-sysex"; name = "help-short"; }
      { tool = "pw-dsdplay"; name = "help-short"; }
      { tool = "pw-encplay"; name = "help-short"; }
      { tool = "pw-cat"; name = "invalid-V"; }
      { tool = "pw-play"; name = "invalid-V"; }
      { tool = "pw-record"; name = "invalid-V"; }
      { tool = "pw-midiplay"; name = "invalid-V"; }
      { tool = "pw-midirecord"; name = "invalid-V"; }
      { tool = "pw-midi2play"; name = "invalid-V"; }
      { tool = "pw-midi2record"; name = "invalid-V"; }
      { tool = "pw-sysex"; name = "invalid-V"; }
      { tool = "pw-dsdplay"; name = "invalid-V"; }
      { tool = "pw-encplay"; name = "invalid-V"; }
      { tool = "pw-cat"; name = "list-formats"; }
      { tool = "pw-dot"; name = "empty-json"; }
      { tool = "pw-cat"; name = "list-channel-names"; }
      { tool = "pw-cat"; name = "list-layouts"; }
      { tool = "pw-cat"; name = "list-containers"; }
      { tool = "pw-reserve"; name = "help"; }
      { tool = "pw-reserve"; name = "version"; }
      { tool = "pw-container"; name = "help"; }
      { tool = "pw-container"; name = "version"; }
      { tool = "pipewire-aes67"; name = "help"; }
      { tool = "pipewire-aes67"; name = "version"; }
      { tool = "pipewire-avb"; name = "help"; }
      { tool = "pipewire-avb"; name = "version"; }
      { tool = "pipewire-vulkan"; name = "help"; }
      { tool = "pipewire-vulkan"; name = "version"; }
      { tool = "pipewire"; name = "bad-flag"; }
      { tool = "pipewire-pulse"; name = "bad-flag"; }
      { tool = "pipewire-aes67"; name = "bad-flag"; }
      { tool = "pipewire-avb"; name = "bad-flag"; }
      { tool = "pipewire-vulkan"; name = "bad-flag"; }
      { tool = "pw-mon"; name = "bad-flag"; }
      { tool = "pw-mididump"; name = "bad-flag"; }
      { tool = "pw-dot"; name = "bad-flag"; }
      { tool = "pw-top"; name = "bad-flag"; }
      { tool = "pw-profiler"; name = "bad-flag"; }
      { tool = "pw-reserve"; name = "bad-flag"; }
      { tool = "pw-container"; name = "bad-flag"; }
      { tool = "pw-loopback"; name = "bad-flag"; }
      { tool = "pw-v4l2"; name = "bad-flag"; }
      { tool = "spa-acp-tool"; name = "bad-flag"; }
      { tool = "spa-resample"; name = "help"; }
      { tool = "spa-resample"; name = "no-args"; }
      { tool = "spa-resample"; name = "one-arg"; }
      { tool = "spa-resample"; name = "bad-flag"; }
      { tool = "pw-v4l2"; name = "help"; }
      { tool = "spa-inspect"; name = "usage"; }
      { tool = "spa-monitor"; name = "usage"; }
      { tool = "spa-inspect"; name = "bad-plugin"; }
      { tool = "spa-monitor"; name = "bad-plugin"; }
      { tool = "spa-acp-tool"; name = "help"; }
      { tool = "spa-acp-tool"; name = "invalid-V"; }
      { tool = "pw-loopback"; name = "help"; }
      { tool = "pw-cli"; name = "version-short"; }
      { tool = "pw-mon"; name = "version-short"; }
      { tool = "pw-link"; name = "version-short"; }
      { tool = "pw-metadata"; name = "version-short"; }
      { tool = "pw-config"; name = "version-short"; }
      { tool = "pw-dump"; name = "version-short"; }
      { tool = "pw-dot"; name = "version-short"; }
      { tool = "pw-mididump"; name = "version-short"; }
      { tool = "pw-profiler"; name = "version-short"; }
      { tool = "pw-top"; name = "version-short"; }
      { tool = "pipewire"; name = "version-short"; }
      { tool = "pipewire-pulse"; name = "version-short"; }
      { tool = "pw-cli"; name = "help-short"; }
      { tool = "pw-mon"; name = "help-short"; }
      { tool = "pw-link"; name = "help-short"; }
      { tool = "pw-metadata"; name = "help-short"; }
      { tool = "pw-config"; name = "help-short"; }
      { tool = "pw-dump"; name = "help-short"; }
      { tool = "pw-dot"; name = "help-short"; }
      { tool = "pw-mididump"; name = "help-short"; }
      { tool = "pw-profiler"; name = "help-short"; }
      { tool = "pw-top"; name = "help-short"; }
      { tool = "pipewire"; name = "help-short"; }
      { tool = "pipewire-pulse"; name = "help-short"; }

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
      {
        tool = "pw-mididump";
        name = "bad-file";
      }
      {
        tool = "pw-mididump";
        name = "bad-force-midi";
      }
      {
        tool = "pw-mididump";
        name = "missing-M-arg";
      }
      {
        tool = "pw-mididump";
        name = "stdin";
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
      {
        tool = "pw-config";
        name = "bad-flag";
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
      {tool = "pw-cli"; name = "ls-module-rich";}
      {tool = "pw-cli"; name = "ls-factory-rich";}
      {tool = "pw-cli"; name = "ls-securitycontext-rich";}
      {tool = "pw-cli"; name = "ls-metadata-rich";}
      {tool = "pw-cli"; name = "info-module-1-rich";}
      {tool = "pw-cli"; name = "info-module-3-rich";}
      {tool = "pw-cli"; name = "info-module-5-rich";}
      {tool = "pw-cli"; name = "info-factory-rich";}
      {tool = "pw-cli"; name = "info-bad-id-rich";}
      {tool = "pw-cli"; name = "info-securitycontext-rich";}
      {tool = "pw-cli"; name = "help-cmd-rich";}
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
      {tool = "pw-cli"; name = "info-by-name-Module";}
      {tool = "pw-cli"; name = "info-by-name-Factory";}
      {tool = "pw-cli"; name = "info-by-name-SecurityContext";}
      {tool = "pw-cli"; name = "info-by-name-Metadata";}
      {tool = "pw-cli"; name = "unknown-command";}
      {tool = "pw-cli"; name = "ls-core-quoted";}
      {tool = "pw-cli"; name = "ls-multiarg";}
      {tool = "pw-cli"; name = "help-with-d";}
      {tool = "pw-cli"; name = "connect-fail";}
      {tool = "pw-cli"; name = "cmd-connect";}
      {tool = "pw-cli"; name = "cmd-disconnect";}
      {tool = "pw-cli"; name = "cmd-switch-remote";}
      {tool = "pw-cli"; name = "bad-flag";}
      {tool = "pw-link"; name = "bad-flag";}
      {tool = "pw-metadata"; name = "bad-flag";}
      {tool = "pw-dump"; name = "bad-flag";}
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
      {tool = "pw-cli"; name = "usage-alias-lm";}
      {tool = "pw-cli"; name = "usage-alias-um";}
      {tool = "pw-cli"; name = "usage-alias-cd";}
      {tool = "pw-cli"; name = "usage-alias-cn";}
      {tool = "pw-cli"; name = "usage-alias-d";}
      {tool = "pw-cli"; name = "usage-alias-cl";}
      {tool = "pw-cli"; name = "usage-alias-en";}
      {tool = "pw-cli"; name = "usage-alias-e";}
      {tool = "pw-cli"; name = "usage-alias-s";}
      {tool = "pw-cli"; name = "usage-alias-sp";}
      {tool = "pw-cli"; name = "usage-alias-gp";}
      {tool = "pw-cli"; name = "usage-alias-c";}
      {tool = "pw-link"; name = "empty-input";}
      {tool = "pw-link"; name = "latency-only";}
      {tool = "pw-link"; name = "empty-output";}
      {tool = "pw-link"; name = "empty-links";}
      {tool = "pw-link"; name = "links-empty-verbose-id";}
      {tool = "pw-link"; name = "connect-fail";}
      {tool = "pw-metadata"; name = "list-empty";}
      {tool = "pw-metadata"; name = "connect-fail";}
      {tool = "pw-dump"; name = "connect-fail";}
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
