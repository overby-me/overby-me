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
    ];
  in
    builtins.listToAttrs (map (t: {
        name = "rust-pipewire-test-${t.tool}-${t.name}";
        value = pkgs:
          import ./testsuite.nix {
            inherit pkgs;
            inherit (t) tool name;
          };
      })
      testDefs);
}
