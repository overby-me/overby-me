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
      {
        tool = "pw-cli";
        name = "version";
      }
      {
        tool = "pw-mon";
        name = "version";
      }
      {
        tool = "pw-link";
        name = "version";
      }
      {
        tool = "pw-metadata";
        name = "version";
      }
      {
        tool = "pw-config";
        name = "version";
      }
      {
        tool = "pw-dump";
        name = "version";
      }
      {
        tool = "pw-dot";
        name = "version";
      }
      {
        tool = "pw-mididump";
        name = "version";
      }
      {
        tool = "pw-profiler";
        name = "version";
      }
      {
        tool = "pw-top";
        name = "version";
      }
      {
        tool = "pipewire";
        name = "version";
      }
      {
        tool = "pipewire-pulse";
        name = "version";
      }
      {
        tool = "pw-cat";
        name = "help";
      }
      {
        tool = "pw-play";
        name = "help";
      }
      {
        tool = "pw-record";
        name = "help";
      }
      {
        tool = "pw-midiplay";
        name = "help";
      }
      {
        tool = "pw-midirecord";
        name = "help";
      }
      {
        tool = "pw-midi2play";
        name = "help";
      }
      {
        tool = "pw-midi2record";
        name = "help";
      }
      {
        tool = "pw-sysex";
        name = "help";
      }
      {
        tool = "pw-dsdplay";
        name = "help";
      }
      {
        tool = "pw-encplay";
        name = "help";
      }
      {
        tool = "pw-cat";
        name = "version";
      }
      {
        tool = "pw-play";
        name = "version";
      }
      {
        tool = "pw-record";
        name = "version";
      }
      {
        tool = "pw-midiplay";
        name = "version";
      }
      {
        tool = "pw-midirecord";
        name = "version";
      }
      {
        tool = "pw-midi2play";
        name = "version";
      }
      {
        tool = "pw-midi2record";
        name = "version";
      }
      {
        tool = "pw-sysex";
        name = "version";
      }
      {
        tool = "pw-dsdplay";
        name = "version";
      }
      {
        tool = "pw-encplay";
        name = "version";
      }
      {
        tool = "pw-cat";
        name = "no-args";
      }
      {
        tool = "pw-play";
        name = "no-args";
      }
      {
        tool = "pw-record";
        name = "no-args";
      }
      {
        tool = "pw-midiplay";
        name = "no-args";
      }
      {
        tool = "pw-midirecord";
        name = "no-args";
      }
      {
        tool = "pw-midi2play";
        name = "no-args";
      }
      {
        tool = "pw-midi2record";
        name = "no-args";
      }
      {
        tool = "pw-sysex";
        name = "no-args";
      }
      {
        tool = "pw-dsdplay";
        name = "no-args";
      }
      {
        tool = "pw-encplay";
        name = "no-args";
      }
      {
        tool = "pw-cat";
        name = "bad-flag";
      }
      {
        tool = "pw-play";
        name = "bad-flag";
      }
      {
        tool = "pw-record";
        name = "bad-flag";
      }
      {
        tool = "pw-midiplay";
        name = "bad-flag";
      }
      {
        tool = "pw-midirecord";
        name = "bad-flag";
      }
      {
        tool = "pw-midi2play";
        name = "bad-flag";
      }
      {
        tool = "pw-midi2record";
        name = "bad-flag";
      }
      {
        tool = "pw-sysex";
        name = "bad-flag";
      }
      {
        tool = "pw-dsdplay";
        name = "bad-flag";
      }
      {
        tool = "pw-encplay";
        name = "bad-flag";
      }
      {
        tool = "pw-cat";
        name = "help-short";
      }
      {
        tool = "pw-play";
        name = "help-short";
      }
      {
        tool = "pw-record";
        name = "help-short";
      }
      {
        tool = "pw-midiplay";
        name = "help-short";
      }
      {
        tool = "pw-midirecord";
        name = "help-short";
      }
      {
        tool = "pw-midi2play";
        name = "help-short";
      }
      {
        tool = "pw-midi2record";
        name = "help-short";
      }
      {
        tool = "pw-sysex";
        name = "help-short";
      }
      {
        tool = "pw-dsdplay";
        name = "help-short";
      }
      {
        tool = "pw-encplay";
        name = "help-short";
      }
      {
        tool = "pw-cat";
        name = "invalid-V";
      }
      {
        tool = "pw-play";
        name = "invalid-V";
      }
      {
        tool = "pw-record";
        name = "invalid-V";
      }
      {
        tool = "pw-midiplay";
        name = "invalid-V";
      }
      {
        tool = "pw-midirecord";
        name = "invalid-V";
      }
      {
        tool = "pw-midi2play";
        name = "invalid-V";
      }
      {
        tool = "pw-midi2record";
        name = "invalid-V";
      }
      {
        tool = "pw-sysex";
        name = "invalid-V";
      }
      {
        tool = "pw-dsdplay";
        name = "invalid-V";
      }
      {
        tool = "pw-encplay";
        name = "invalid-V";
      }
      {
        tool = "pw-cat";
        name = "list-formats";
      }
      {
        tool = "pw-dot";
        name = "empty-json";
      }
      {
        tool = "pw-cat";
        name = "list-channel-names";
      }
      {
        tool = "pw-cat";
        name = "list-layouts";
      }
      {
        tool = "pw-cat";
        name = "list-containers";
      }
      {
        tool = "pw-reserve";
        name = "help";
      }
      {
        tool = "pw-reserve";
        name = "version";
      }
      {
        tool = "pw-container";
        name = "help";
      }
      {
        tool = "pw-container";
        name = "version";
      }
      {
        tool = "pipewire-aes67";
        name = "help";
      }
      {
        tool = "pipewire-aes67";
        name = "version";
      }
      {
        tool = "pipewire-avb";
        name = "help";
      }
      {
        tool = "pipewire-avb";
        name = "version";
      }
      {
        tool = "pipewire-vulkan";
        name = "help";
      }
      {
        tool = "pipewire-vulkan";
        name = "version";
      }
      {
        tool = "pipewire-aes67";
        name = "help-short";
      }
      {
        tool = "pipewire-aes67";
        name = "version-short";
      }
      {
        tool = "pipewire-avb";
        name = "help-short";
      }
      {
        tool = "pipewire-avb";
        name = "version-short";
      }
      {
        tool = "pipewire-vulkan";
        name = "help-short";
      }
      {
        tool = "pipewire-vulkan";
        name = "version-short";
      }
      {
        tool = "pw-reserve";
        name = "help-short";
      }
      {
        tool = "pw-reserve";
        name = "version-short";
      }
      {
        tool = "pw-container";
        name = "help-short";
      }
      {
        tool = "pw-container";
        name = "version-short";
      }
      {
        tool = "pw-loopback";
        name = "help-short";
      }
      {
        tool = "pipewire";
        name = "bad-flag";
      }
      {
        tool = "pipewire-pulse";
        name = "bad-flag";
      }
      {
        tool = "pipewire-aes67";
        name = "bad-flag";
      }
      {
        tool = "pipewire-avb";
        name = "bad-flag";
      }
      {
        tool = "pipewire-vulkan";
        name = "bad-flag";
      }
      {
        tool = "pw-mon";
        name = "bad-flag";
      }
      {
        tool = "pw-mididump";
        name = "bad-flag";
      }
      {
        tool = "pw-dot";
        name = "bad-flag";
      }
      {
        tool = "pw-top";
        name = "bad-flag";
      }
      {
        tool = "pw-profiler";
        name = "bad-flag";
      }
      {
        tool = "pw-reserve";
        name = "bad-flag";
      }
      {
        tool = "pw-container";
        name = "bad-flag";
      }
      {
        tool = "pw-loopback";
        name = "bad-flag";
      }
      {
        tool = "pw-v4l2";
        name = "bad-flag";
      }
      {
        tool = "spa-acp-tool";
        name = "bad-flag";
      }
      {
        tool = "spa-resample";
        name = "help";
      }
      {
        tool = "spa-resample";
        name = "no-args";
      }
      {
        tool = "spa-resample";
        name = "one-arg";
      }
      {
        tool = "spa-resample";
        name = "bad-flag";
      }
      {
        tool = "spa-resample";
        name = "invalid-V";
      }
      {
        tool = "spa-resample";
        name = "missing-arg-c";
      }
      {
        tool = "spa-resample";
        name = "missing-arg-r";
      }
      {
        tool = "spa-resample";
        name = "with-c-flag";
      }
      {
        tool = "pw-v4l2";
        name = "help";
      }
      {
        tool = "spa-inspect";
        name = "usage";
      }
      {
        tool = "spa-monitor";
        name = "usage";
      }
      {
        tool = "spa-inspect";
        name = "bad-plugin";
      }
      {
        tool = "spa-monitor";
        name = "bad-plugin";
      }
      {
        tool = "spa-acp-tool";
        name = "help";
      }
      {
        tool = "spa-acp-tool";
        name = "invalid-V";
      }
      {
        tool = "pw-loopback";
        name = "help";
      }
      {
        tool = "pw-cli";
        name = "version-short";
      }
      {
        tool = "pw-mon";
        name = "version-short";
      }
      {
        tool = "pw-link";
        name = "version-short";
      }
      {
        tool = "pw-metadata";
        name = "version-short";
      }
      {
        tool = "pw-config";
        name = "version-short";
      }
      {
        tool = "pw-dump";
        name = "version-short";
      }
      {
        tool = "pw-dot";
        name = "version-short";
      }
      {
        tool = "pw-mididump";
        name = "version-short";
      }
      {
        tool = "pw-profiler";
        name = "version-short";
      }
      {
        tool = "pw-top";
        name = "version-short";
      }
      {
        tool = "pipewire";
        name = "version-short";
      }
      {
        tool = "pipewire-pulse";
        name = "version-short";
      }
      {
        tool = "pw-cli";
        name = "help-short";
      }
      {
        tool = "pw-mon";
        name = "help-short";
      }
      {
        tool = "pw-link";
        name = "help-short";
      }
      {
        tool = "pw-metadata";
        name = "help-short";
      }
      {
        tool = "pw-config";
        name = "help-short";
      }
      {
        tool = "pw-dump";
        name = "help-short";
      }
      {
        tool = "pw-dot";
        name = "help-short";
      }
      {
        tool = "pw-mididump";
        name = "help-short";
      }
      {
        tool = "pw-profiler";
        name = "help-short";
      }
      {
        tool = "pw-top";
        name = "help-short";
      }
      {
        tool = "pipewire";
        name = "help-short";
      }
      {
        tool = "pipewire-pulse";
        name = "help-short";
      }
      {
        tool = "pw-cli";
        name = "short-bad-flag";
      }
      {
        tool = "pw-mon";
        name = "short-bad-flag";
      }
      {
        tool = "pw-link";
        name = "short-bad-flag";
      }
      {
        tool = "pw-metadata";
        name = "short-bad-flag";
      }
      {
        tool = "pw-dump";
        name = "short-bad-flag";
      }
      {
        tool = "pw-dot";
        name = "short-bad-flag";
      }
      {
        tool = "pw-profiler";
        name = "short-bad-flag";
      }
      {
        tool = "pw-top";
        name = "short-bad-flag";
      }
      {
        tool = "pw-cli";
        name = "missing-arg-r";
      }
      {
        tool = "pw-link";
        name = "missing-arg-r";
      }
      {
        tool = "pw-link";
        name = "missing-arg-p";
      }
      {
        tool = "pw-mon";
        name = "missing-arg-r";
      }
      {
        tool = "pw-top";
        name = "missing-arg-r";
      }
      {
        tool = "pw-top";
        name = "missing-arg-n";
      }
      {
        tool = "pw-dump";
        name = "missing-arg-r";
      }
      {
        tool = "pw-dump";
        name = "missing-arg-i";
      }
      {
        tool = "pw-dot";
        name = "missing-arg-r";
      }
      {
        tool = "pw-dot";
        name = "missing-arg-j";
      }
      {
        tool = "pw-dot";
        name = "missing-arg-o";
      }
      {
        tool = "pw-metadata";
        name = "missing-arg-r";
      }
      {
        tool = "pw-metadata";
        name = "missing-arg-n";
      }
      {
        tool = "pw-cat";
        name = "missing-arg-R";
      }
      {
        tool = "pw-cat";
        name = "missing-arg-P";
      }
      {
        tool = "pw-cat";
        name = "missing-arg-q";
      }
      {
        tool = "pw-cat";
        name = "missing-arg-M";
      }
      {
        tool = "pw-cat";
        name = "missing-arg-n";
      }
      {
        tool = "pw-cat";
        name = "missing-arg-rate";
      }
      {
        tool = "pw-cat";
        name = "missing-arg-target";
      }
      {
        tool = "pw-cat";
        name = "missing-arg-media-type";
      }
      {
        tool = "pw-cat";
        name = "playback-no-file";
      }
      {
        tool = "pw-cat";
        name = "record-no-file";
      }
      {
        tool = "pw-cli";
        name = "cluster-hV";
      }
      {
        tool = "pw-mon";
        name = "cluster-hV";
      }
      {
        tool = "pw-link";
        name = "cluster-hV";
      }
      {
        tool = "pw-top";
        name = "cluster-hV";
      }
      {
        tool = "pw-dump";
        name = "cluster-hV";
      }
      {
        tool = "pw-dot";
        name = "cluster-hV";
      }
      {
        tool = "pw-metadata";
        name = "cluster-hV";
      }
      {
        tool = "pw-profiler";
        name = "cluster-hV";
      }
      {
        tool = "pw-config";
        name = "cluster-hV";
      }
      {
        tool = "pw-mididump";
        name = "cluster-hV";
      }
      {
        tool = "pw-cat";
        name = "cluster-hp";
      }
      {
        tool = "pw-cat";
        name = "cluster-Vh";
      }
      {
        tool = "pw-cli";
        name = "help-with-arg";
      }
      {
        tool = "pw-cli";
        name = "version-with-arg";
      }
      {
        tool = "pw-mon";
        name = "help-with-arg";
      }
      {
        tool = "pw-mon";
        name = "version-with-arg";
      }
      {
        tool = "pw-link";
        name = "help-with-arg";
      }
      {
        tool = "pw-link";
        name = "version-with-arg";
      }
      {
        tool = "pw-top";
        name = "help-with-arg";
      }
      {
        tool = "pw-top";
        name = "version-with-arg";
      }
      {
        tool = "pw-dump";
        name = "help-with-arg";
      }
      {
        tool = "pw-dump";
        name = "version-with-arg";
      }
      {
        tool = "pw-dot";
        name = "help-with-arg";
      }
      {
        tool = "pw-dot";
        name = "version-with-arg";
      }
      {
        tool = "pw-metadata";
        name = "help-with-arg";
      }
      {
        tool = "pw-metadata";
        name = "version-with-arg";
      }
      {
        tool = "pw-profiler";
        name = "help-with-arg";
      }
      {
        tool = "pw-profiler";
        name = "version-with-arg";
      }
      {
        tool = "pw-cat";
        name = "help-with-arg";
      }
      {
        tool = "pw-cat";
        name = "version-with-arg";
      }
      {
        tool = "pw-play";
        name = "help-with-arg";
      }
      {
        tool = "pw-play";
        name = "version-with-arg";
      }
      {
        tool = "pw-record";
        name = "help-with-arg";
      }
      {
        tool = "pw-record";
        name = "version-with-arg";
      }
      {
        tool = "pw-midiplay";
        name = "help-with-arg";
      }
      {
        tool = "pw-midiplay";
        name = "version-with-arg";
      }
      {
        tool = "pw-midirecord";
        name = "help-with-arg";
      }
      {
        tool = "pw-midirecord";
        name = "version-with-arg";
      }
      {
        tool = "pw-midi2play";
        name = "help-with-arg";
      }
      {
        tool = "pw-midi2play";
        name = "version-with-arg";
      }
      {
        tool = "pw-midi2record";
        name = "help-with-arg";
      }
      {
        tool = "pw-midi2record";
        name = "version-with-arg";
      }
      {
        tool = "pw-sysex";
        name = "help-with-arg";
      }
      {
        tool = "pw-sysex";
        name = "version-with-arg";
      }
      {
        tool = "pw-dsdplay";
        name = "help-with-arg";
      }
      {
        tool = "pw-dsdplay";
        name = "version-with-arg";
      }
      {
        tool = "pw-encplay";
        name = "help-with-arg";
      }
      {
        tool = "pw-encplay";
        name = "version-with-arg";
      }
      {
        tool = "pw-config";
        name = "help-with-arg";
      }
      {
        tool = "pw-config";
        name = "version-with-arg";
      }
      {
        tool = "pw-mididump";
        name = "help-with-arg";
      }
      {
        tool = "pw-mididump";
        name = "version-with-arg";
      }
      {
        tool = "pw-loopback";
        name = "help-with-arg";
      }
      {
        tool = "pw-loopback";
        name = "version-with-arg";
      }
      {
        tool = "pw-reserve";
        name = "help-with-arg";
      }
      {
        tool = "pw-reserve";
        name = "version-with-arg";
      }
      {
        tool = "pw-container";
        name = "help-with-arg";
      }
      {
        tool = "pw-container";
        name = "version-with-arg";
      }
      {
        tool = "pipewire";
        name = "help-with-arg";
      }
      {
        tool = "pipewire";
        name = "version-with-arg";
      }
      {
        tool = "pipewire-pulse";
        name = "help-with-arg";
      }
      {
        tool = "pipewire-pulse";
        name = "version-with-arg";
      }
      {
        tool = "pipewire-aes67";
        name = "help-with-arg";
      }
      {
        tool = "pipewire-aes67";
        name = "version-with-arg";
      }
      {
        tool = "pipewire-avb";
        name = "help-with-arg";
      }
      {
        tool = "pipewire-avb";
        name = "version-with-arg";
      }
      {
        tool = "pipewire-vulkan";
        name = "help-with-arg";
      }
      {
        tool = "pipewire-vulkan";
        name = "version-with-arg";
      }
      {
        tool = "spa-json-dump";
        name = "help-with-arg";
      }
      {
        tool = "spa-acp-tool";
        name = "help-with-arg";
      }
      {
        tool = "spa-resample";
        name = "help-with-arg";
      }
      {
        tool = "pw-config";
        name = "merge-no-section";
      }
      {
        tool = "pw-config";
        name = "paths-recurse";
      }
      {
        tool = "pw-cli";
        name = "info-alias-i-bad";
      }
      {
        tool = "pw-cli";
        name = "info-bad-exit-code";
      }
      {
        tool = "pw-cli";
        name = "info-multi-arg";
      }
      {
        tool = "pw-cli";
        name = "destroy-bad";
      }
      {
        tool = "pw-cli";
        name = "destroy-alias-bad";
      }
      {
        tool = "pw-cli";
        name = "load-module-bad";
      }
      {
        tool = "pw-cli";
        name = "unload-module-bad";
      }
      {
        tool = "pw-cli";
        name = "um-alias-bad";
      }
      {
        tool = "pw-cli";
        name = "create-device-bad";
      }
      {
        tool = "pw-cli";
        name = "create-node-bad";
      }
      {
        tool = "pw-cli";
        name = "enum-params-bad";
      }
      {
        tool = "pw-cli";
        name = "set-param-bad";
      }
      {
        tool = "pw-cli";
        name = "send-command-bad";
      }
      {
        tool = "pw-cli";
        name = "get-permissions-bad";
      }
      {
        tool = "pw-cli";
        name = "create-link-bad";
      }
      {
        tool = "pw-cli";
        name = "export-node-bad";
      }
      {
        tool = "pw-mon";
        name = "invalid-color";
      }
      {
        tool = "pw-link";
        name = "color-rejected";
      }
      {
        tool = "pw-dump";
        name = "invalid-color";
      }
      {
        tool = "pw-link";
        name = "latency-with-arg";
      }
      {
        tool = "pw-link";
        name = "input-with-arg";
      }
      {
        tool = "pw-mon";
        name = "no-colors-with-arg";
      }
      {
        tool = "pw-dump";
        name = "no-colors-with-arg";
      }
      {
        tool = "pw-dump";
        name = "monitor-with-arg";
      }
      {
        tool = "pw-top";
        name = "batch-mode-with-arg";
      }
      {
        tool = "pw-dot";
        name = "all-with-arg";
      }
      {
        tool = "pw-dot";
        name = "lr-with-arg";
      }
      {
        tool = "pw-metadata";
        name = "monitor-with-arg";
      }
      {
        tool = "pw-metadata";
        name = "list-with-arg";
      }
      {
        tool = "pw-cat";
        name = "raw-with-arg";
      }
      {
        tool = "pw-cat";
        name = "verbose-with-arg";
      }
      {
        tool = "pw-play";
        name = "raw-with-arg";
      }
      {
        tool = "pw-play";
        name = "verbose-with-arg";
      }
      {
        tool = "pw-record";
        name = "raw-with-arg";
      }
      {
        tool = "pw-record";
        name = "verbose-with-arg";
      }
      {
        tool = "pw-config";
        name = "recurse-with-arg";
      }
      {
        tool = "pw-config";
        name = "no-newline-with-arg";
      }
      {
        tool = "pw-loopback";
        name = "remote-with-arg";
      }
      {
        tool = "spa-acp-tool";
        name = "verbose-with-arg";
      }
      {
        tool = "spa-resample";
        name = "verbose-with-arg";
      }
      {
        tool = "spa-resample";
        name = "rate-inline";
      }
      {
        tool = "pw-mididump";
        name = "multi-track";
      }
      {
        tool = "pw-mididump";
        name = "key-signature";
      }
      {
        tool = "pw-mididump";
        name = "smpte-offset";
      }
      {
        tool = "pw-mididump";
        name = "midi-channel-prefix";
      }
      {
        tool = "spa-json-dump";
        name = "escaped-strings";
      }
      {
        tool = "spa-json-dump";
        name = "deep-nesting";
      }
      {
        tool = "spa-json-dump";
        name = "array-of-arrays";
      }
      {
        tool = "pw-mididump";
        name = "sequence-number";
      }
      {
        tool = "pw-mididump";
        name = "pitch-wheel";
      }
      { tool = "pw-cli"; name = "remote-inline-eq"; }
      { tool = "pw-cli"; name = "remote-short-eq"; }
      { tool = "pw-cli"; name = "remote-short-attached"; }
      { tool = "pw-mon"; name = "remote-bad"; }
      { tool = "pw-link"; name = "remote-bad"; }
      { tool = "pw-dump"; name = "remote-bad"; }
      { tool = "pw-dot"; name = "remote-attached"; }
      { tool = "pw-metadata"; name = "remote-bad"; }
      { tool = "pw-cli"; name = "help-with-bad-remote"; }
      { tool = "spa-acp-tool"; name = "cluster-hv"; }
      { tool = "pw-cat"; name = "dash-dash-only"; }
      { tool = "pw-mididump"; name = "multi-positional"; }
      { tool = "spa-resample"; name = "two-positional-fail"; }
      { tool = "pw-cat"; name = "playback-bad-file"; }
      { tool = "pw-play"; name = "bad-file"; }
      { tool = "pw-record"; name = "bad-file"; }
      { tool = "pw-mididump"; name = "live-no-daemon"; }
      { tool = "pw-config"; name = "dash-dash-paths"; }
      { tool = "pw-cli"; name = "info-no-args"; }
      { tool = "pw-cli"; name = "i-alias-no-args"; }
      { tool = "pw-cli"; name = "ls-dash"; }
      { tool = "spa-acp-tool"; name = "unknown-cmd"; }
      { tool = "pw-link"; name = "props-attached"; }
      { tool = "pw-link"; name = "props-empty"; }
      { tool = "pw-config"; name = "invalid-color"; }
      { tool = "pw-link"; name = "latency-with-pos"; }
      { tool = "spa-json-dump"; name = "conf-filter-chain"; }
      { tool = "spa-json-dump"; name = "conf-pipewire-vulkan"; }
      { tool = "spa-json-dump"; name = "conf-pipewire-pulse"; }
      { tool = "spa-json-dump"; name = "conf-pipewire-avb"; }
      { tool = "spa-json-dump"; name = "conf-fc-22-onnx-vad"; }
      { tool = "spa-json-dump"; name = "conf-fc-35-ebur128"; }
      { tool = "spa-json-dump"; name = "conf-fc-36-dcblock"; }
      { tool = "spa-json-dump"; name = "conf-fc-demonic"; }
      { tool = "spa-json-dump"; name = "conf-fc-sink-dolby-pro-logic-ii"; }
      { tool = "spa-json-dump"; name = "conf-fc-sink-eq6"; }
      { tool = "pw-mididump"; name = "M-attached-bad"; }
      { tool = "pw-mididump"; name = "M-equals-ump"; }
      { tool = "pw-mididump"; name = "M-ump-file"; }
      { tool = "pw-config"; name = "short-bad-flag"; }
      { tool = "spa-resample"; name = "short-bad-cluster"; }
      { tool = "spa-acp-tool"; name = "short-bad-cluster"; }
      { tool = "spa-json-dump"; name = "short-bad-cluster"; }
      { tool = "pw-link"; name = "short-bad-cluster"; }
      { tool = "pw-mon"; name = "short-bad-cluster"; }
      { tool = "pw-dump"; name = "short-bad-cluster"; }
      { tool = "pw-dot"; name = "short-bad-cluster"; }
      { tool = "pw-metadata"; name = "short-bad-cluster"; }
      { tool = "pw-profiler"; name = "short-bad-cluster"; }
      { tool = "pw-mididump"; name = "short-bad-cluster"; }
      { tool = "pw-loopback"; name = "short-bad-cluster"; }
      { tool = "pw-container"; name = "short-bad-cluster"; }
      { tool = "pw-reserve"; name = "short-bad-cluster"; }
      { tool = "pw-mon"; name = "C-cluster-attached"; }
      { tool = "pw-top"; name = "short-bad-cluster"; }
      { tool = "pw-cli"; name = "pipewire-remote-bad-info"; }
      { tool = "pw-cli"; name = "pipewire-remote-bad-quit"; }
      { tool = "pw-cli"; name = "pipewire-remote-bad-listvars"; }
      { tool = "pw-cli"; name = "connect-refused-info"; }
      { tool = "pw-cli"; name = "connect-refused-help"; }
      { tool = "pw-cat"; name = "playback-existing-bad-format"; }
      { tool = "pw-cat"; name = "playback-stdin-marker"; }
      { tool = "spa-resample"; name = "existing-bad-format"; }
      { tool = "spa-resample"; name = "dash-stdin-marker"; }
      { tool = "pw-mididump"; name = "directory-arg"; }
      { tool = "spa-json-dump"; name = "stdin-marker"; }
      { tool = "spa-json-dump"; name = "no-args"; }
      { tool = "spa-json-dump"; name = "empty-file"; }
      { tool = "spa-json-dump"; name = "directory"; }
      { tool = "pw-cat"; name = "bad-rate-string"; }
      { tool = "pw-cat"; name = "bad-rate-zero"; }
      { tool = "pw-cat"; name = "bad-rate-negative"; }
      { tool = "pw-cat"; name = "bad-channels-string"; }
      { tool = "pw-mididump"; name = "dash-dash-terminator"; }
      { tool = "pw-mididump"; name = "dash-dash-empty"; }
      { tool = "spa-resample"; name = "dash-dash-only"; }
      { tool = "spa-resample"; name = "dash-dash-args"; }
      { tool = "spa-json-dump"; name = "dash-dash-only"; }
      { tool = "spa-json-dump"; name = "dash-dash-missing-file"; }
      { tool = "spa-json-dump"; name = "dash-dash-directory"; }
      { tool = "spa-json-dump"; name = "proc-zero-size"; }
      { tool = "spa-json-dump"; name = "stdin-content"; }
      { tool = "spa-json-dump"; name = "stdin-whitespace-only"; }
      { tool = "spa-json-dump"; name = "stdin-comment-only"; }
      { tool = "spa-json-dump"; name = "bare-number-stdin"; }
      { tool = "spa-json-dump"; name = "bare-string-stdin"; }
      { tool = "spa-json-dump"; name = "bare-bool-stdin"; }
      { tool = "spa-resample"; name = "bad-rate-string"; }
      { tool = "spa-resample"; name = "bad-rate-long"; }
      { tool = "spa-resample"; name = "bad-format"; }
      { tool = "spa-resample"; name = "bad-format-long"; }
      { tool = "spa-resample"; name = "bad-quality-negative"; }
      { tool = "spa-resample"; name = "attached-rate"; }
      { tool = "spa-resample"; name = "long-rate-no-arg"; }
      { tool = "spa-resample"; name = "long-format-no-arg"; }
      { tool = "spa-resample"; name = "long-quality-no-arg"; }
      { tool = "spa-json-dump"; name = "indent-no-arg"; }
      { tool = "spa-json-dump"; name = "i-short-no-arg"; }
      { tool = "pw-cli"; name = "long-remote-no-arg"; }
      { tool = "pw-link"; name = "long-remote-no-arg"; }
      { tool = "pw-mon"; name = "long-remote-no-arg"; }
      { tool = "pw-mididump"; name = "long-remote-no-arg"; }
      { tool = "pw-mididump"; name = "short-remote-no-arg"; }
      { tool = "pw-config"; name = "long-name-no-arg"; }
      { tool = "pw-config"; name = "short-name-no-arg"; }
      { tool = "pw-config"; name = "long-prefix-no-arg"; }
      { tool = "pw-config"; name = "short-prefix-no-arg"; }
      { tool = "spa-json-dump"; name = "cluster-help-shortcircuit"; }
      { tool = "spa-json-dump"; name = "cluster-s-then-h"; }
      { tool = "pw-cli"; name = "env-pipewire-remote"; }
      { tool = "pw-mon"; name = "env-pipewire-remote"; }
      { tool = "pw-mididump"; name = "env-pipewire-remote"; }
      { tool = "pw-link"; name = "env-pipewire-remote"; }
      { tool = "pw-link"; name = "env-pipewire-remote-list"; }
      { tool = "pw-cat"; name = "env-pipewire-remote"; }
      { tool = "pw-cli"; name = "connect-refused"; }
      { tool = "pw-link"; name = "connect-refused"; }
      { tool = "pw-mon"; name = "connect-refused"; }
      { tool = "pw-dump"; name = "connect-refused"; }
      { tool = "pw-dot"; name = "connect-refused"; }
      { tool = "pw-metadata"; name = "connect-refused"; }
      { tool = "pw-top"; name = "connect-refused"; }
      { tool = "pw-profiler"; name = "connect-refused"; }
      { tool = "pw-loopback"; name = "connect-refused"; }
      { tool = "pw-container"; name = "connect-refused"; }
      { tool = "pw-cli"; name = "pipewire-remote-bad"; }
      { tool = "pw-cli"; name = "cluster-hxx"; }
      { tool = "pw-cli"; name = "cluster-Vxx"; }
      { tool = "pw-link"; name = "cluster-hxx"; }
      { tool = "pw-link"; name = "cluster-Vxx"; }
      { tool = "pw-mon"; name = "cluster-hxx"; }
      { tool = "pw-mon"; name = "cluster-Vxx"; }
      { tool = "pw-dump"; name = "cluster-hxx"; }
      { tool = "pw-dump"; name = "cluster-Vxx"; }
      { tool = "pw-dot"; name = "cluster-hxx"; }
      { tool = "pw-dot"; name = "cluster-Vxx"; }
      { tool = "pw-metadata"; name = "cluster-hxx"; }
      { tool = "pw-metadata"; name = "cluster-Vxx"; }
      { tool = "pw-top"; name = "cluster-hxx"; }
      { tool = "pw-top"; name = "cluster-Vxx"; }
      { tool = "pw-profiler"; name = "cluster-hxx"; }
      { tool = "pw-profiler"; name = "cluster-Vxx"; }
      { tool = "pw-config"; name = "cluster-hxx"; }
      { tool = "pw-config"; name = "cluster-Vxx"; }
      { tool = "pw-mididump"; name = "cluster-hxx"; }
      { tool = "pw-mididump"; name = "cluster-Vxx"; }
      { tool = "pw-link"; name = "lone-dash"; }
      { tool = "pw-mon"; name = "lone-dash"; }
      { tool = "pw-link"; name = "d-then-i"; }
      { tool = "pw-link"; name = "i-then-d"; }
      { tool = "pw-link"; name = "I-without-list"; }
      { tool = "pw-config"; name = "list-empty-no-newline"; }
      {
        tool = "pw-cli";
        name = "cmd-connect-bad";
      }
      {
        tool = "pw-mon";
        name = "connect-fail";
      }
      {
        tool = "pw-profiler";
        name = "connect-fail";
      }
      {
        tool = "pw-container";
        name = "connect-fail";
      }
      {
        tool = "pw-loopback";
        name = "connect-fail";
      }
      {
        tool = "pw-reserve";
        name = "no-name";
      }
      {
        tool = "pw-reserve";
        name = "positional-bad";
      }
      {
        tool = "pw-link";
        name = "dash-dash-only";
      }
      {
        tool = "pw-link";
        name = "dash-dash-then-flag";
      }
      {
        tool = "pw-top";
        name = "dash-dash-only";
      }
      {
        tool = "pw-dot";
        name = "dash-dash-only";
      }
      {
        tool = "pw-dump";
        name = "dash-dash-only";
      }
      {
        tool = "pw-metadata";
        name = "dash-dash-only";
      }
      {
        tool = "pw-link";
        name = "connect-three-args";
      }
      {
        tool = "pw-link";
        name = "connect-one-arg";
      }
      {
        tool = "pw-link";
        name = "disconnect-with-id";
      }
      {
        tool = "pw-link";
        name = "connect-two-args";
      }
      {
        tool = "pw-mididump";
        name = "force-midi-bad-value";
      }
      {
        tool = "spa-resample";
        name = "cluster-hh";
      }
      {
        tool = "spa-resample";
        name = "double-v";
      }
      {
        tool = "pw-mididump";
        name = "empty-file";
      }
      {
        tool = "pw-mididump";
        name = "positional-then-bad";
      }
      {
        tool = "pw-v4l2";
        name = "missing-arg-r";
      }
      {
        tool = "pw-v4l2";
        name = "cluster-vh";
      }
      {
        tool = "pw-v4l2";
        name = "no-args";
      }
      {
        tool = "spa-inspect";
        name = "help-flag";
      }
      {
        tool = "spa-inspect";
        name = "two-args";
      }
      {
        tool = "spa-monitor";
        name = "help-flag";
      }
      {
        tool = "spa-monitor";
        name = "two-args";
      }
      {
        tool = "spa-acp-tool";
        name = "missing-arg-c";
      }
      {
        tool = "spa-acp-tool";
        name = "missing-arg-p";
      }
      {
        tool = "pw-mididump";
        name = "sysex";
      }
      {
        tool = "pw-mididump";
        name = "poly-pressure";
      }
      {
        tool = "pw-mididump";
        name = "copyright-meta";
      }
      {
        tool = "spa-json-dump";
        name = "array-mixed";
      }
      {
        tool = "spa-json-dump";
        name = "empty-object";
      }
      {
        tool = "spa-json-dump";
        name = "special-keys";
      }
      {
        tool = "spa-json-dump";
        name = "dash-stdin";
      }
      {
        tool = "spa-json-dump";
        name = "booleans";
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
      {
        tool = "pw-cli";
        name = "ls-node-rich";
      }
      {
        tool = "pw-cli";
        name = "ls-port-rich";
      }
      {
        tool = "pw-cli";
        name = "info-node-rich";
      }
      {
        tool = "pw-cli";
        name = "info-all-rich";
      }
      {
        tool = "pw-cli";
        name = "ls-substring-rich";
      }
      {
        tool = "pw-cli";
        name = "ls-no-filter-rich";
      }
      {
        tool = "pw-cli";
        name = "info-by-name-rich";
      }
      {
        tool = "pw-link";
        name = "input-rich";
      }
      {
        tool = "pw-link";
        name = "output-rich";
      }
      {
        tool = "pw-link";
        name = "input-id-rich";
      }
      {
        tool = "pw-link";
        name = "input-verbose-rich";
      }
      {
        tool = "pw-link";
        name = "input-links-rich";
      }
      {
        tool = "pw-link";
        name = "output-links-rich";
      }
      {
        tool = "pw-link";
        name = "all-flags-rich";
      }
      {
        tool = "pw-link";
        name = "output-verbose-rich";
      }
      {
        tool = "pw-link";
        name = "all-flags-verbose-rich";
      }
      {
        tool = "pw-link";
        name = "input-id-verbose-rich";
      }
      {
        tool = "pw-link";
        name = "pattern-output-empty";
      }
      {
        tool = "pw-link";
        name = "pattern-both-empty";
      }
      {
        tool = "pw-cli";
        name = "ls-module-rich";
      }
      {
        tool = "pw-cli";
        name = "ls-factory-rich";
      }
      {
        tool = "pw-cli";
        name = "ls-securitycontext-rich";
      }
      {
        tool = "pw-cli";
        name = "ls-metadata-rich";
      }
      {
        tool = "pw-cli";
        name = "info-module-1-rich";
      }
      {
        tool = "pw-cli";
        name = "info-module-3-rich";
      }
      {
        tool = "pw-cli";
        name = "info-module-5-rich";
      }
      {
        tool = "pw-cli";
        name = "info-factory-rich";
      }
      {
        tool = "pw-cli";
        name = "info-bad-id-rich";
      }
      {
        tool = "pw-cli";
        name = "info-securitycontext-rich";
      }
      {
        tool = "pw-cli";
        name = "info-by-name-Port";
      }
      {
        tool = "pw-cli";
        name = "help-cmd-rich";
      }
      {
        tool = "pw-metadata";
        name = "list-rich";
      }
      {
        tool = "pw-metadata";
        name = "list-byname";
      }
    ];

    # Daemon-comparison tests — spawn a real C pipewire daemon and run both
    # the C tool and the Rust tool against it, then diff. Each entry is
    # `tools/<tool>/<name>.sh`.
    daemonTests = [
      {
        tool = "pw-cli";
        name = "ls-core";
      }
      {
        tool = "pw-cli";
        name = "ls-module";
      }
      {
        tool = "pw-cli";
        name = "ls-factory";
      }
      {
        tool = "pw-cli";
        name = "ls-securitycontext";
      }
      {
        tool = "pw-cli";
        name = "ls-metadata";
      }
      {
        tool = "pw-cli";
        name = "ls-empty-node";
      }
      {
        tool = "pw-cli";
        name = "ls-empty-link";
      }
      {
        tool = "pw-cli";
        name = "ls-empty-port";
      }
      {
        tool = "pw-cli";
        name = "ls-empty-device";
      }
      {
        tool = "pw-cli";
        name = "ls-all-normalized";
      }
      {
        tool = "pw-cli";
        name = "ls-substring";
      }
      {
        tool = "pw-cli";
        name = "info-core";
      }
      {
        tool = "pw-cli";
        name = "info-module-1";
      }
      {
        tool = "pw-cli";
        name = "info-module-3";
      }
      {
        tool = "pw-cli";
        name = "info-module-5";
      }
      {
        tool = "pw-cli";
        name = "info-factory";
      }
      {
        tool = "pw-cli";
        name = "info-all";
      }
      {
        tool = "pw-cli";
        name = "info-bad-id";
      }
      {
        tool = "pw-cli";
        name = "info-securitycontext";
      }
      {
        tool = "pw-cli";
        name = "info-by-name";
      }
      {
        tool = "pw-cli";
        name = "info-by-name-Module";
      }
      {
        tool = "pw-cli";
        name = "info-by-name-Factory";
      }
      {
        tool = "pw-cli";
        name = "info-by-name-SecurityContext";
      }
      {
        tool = "pw-cli";
        name = "info-by-name-Metadata";
      }
      {
        tool = "pw-cli";
        name = "info-by-name-NonExistent";
      }
      {
        tool = "pw-cli";
        name = "unknown-command";
      }
      {
        tool = "pw-cli";
        name = "ls-core-quoted";
      }
      {
        tool = "pw-cli";
        name = "ls-multiarg";
      }
      {
        tool = "pw-cli";
        name = "help-with-d";
      }
      {
        tool = "pw-cli";
        name = "ls-help-mixed";
      }
      {
        tool = "pw-cli";
        name = "connect-fail";
      }
      {
        tool = "pw-cli";
        name = "cmd-connect";
      }
      {
        tool = "pw-cli";
        name = "cmd-disconnect";
      }
      {
        tool = "pw-cli";
        name = "cmd-switch-remote";
      }
      {
        tool = "pw-cli";
        name = "bad-flag";
      }
      {
        tool = "pw-link";
        name = "bad-flag";
      }
      {
        tool = "pw-metadata";
        name = "bad-flag";
      }
      {
        tool = "pw-dump";
        name = "bad-flag";
      }
      {
        tool = "pw-cli";
        name = "help-cmd";
      }
      {
        tool = "pw-cli";
        name = "list-vars";
      }
      {
        tool = "pw-cli";
        name = "list-remotes";
      }
      {
        tool = "pw-cli";
        name = "quit";
      }
      {
        tool = "pw-cli";
        name = "usage-load-module";
      }
      {
        tool = "pw-cli";
        name = "usage-unload-module";
      }
      {
        tool = "pw-cli";
        name = "usage-create-device";
      }
      {
        tool = "pw-cli";
        name = "usage-create-node";
      }
      {
        tool = "pw-cli";
        name = "usage-destroy";
      }
      {
        tool = "pw-cli";
        name = "usage-enum-params";
      }
      {
        tool = "pw-cli";
        name = "usage-set-param";
      }
      {
        tool = "pw-cli";
        name = "usage-permissions";
      }
      {
        tool = "pw-cli";
        name = "usage-send-command";
      }
      {
        tool = "pw-cli";
        name = "usage-get-permissions";
      }
      {
        tool = "pw-cli";
        name = "usage-create-link";
      }
      {
        tool = "pw-cli";
        name = "usage-export-node";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-lm";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-um";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-cd";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-cn";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-d";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-cl";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-en";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-e";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-s";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-sp";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-gp";
      }
      {
        tool = "pw-cli";
        name = "usage-alias-c";
      }
      {
        tool = "pw-link";
        name = "empty-input";
      }
      {
        tool = "pw-link";
        name = "latency-only";
      }
      {
        tool = "pw-link";
        name = "disconnect-no-args";
      }
      {
        tool = "pw-link";
        name = "connect-missing-input";
      }
      {
        tool = "pw-link";
        name = "empty-output";
      }
      {
        tool = "pw-link";
        name = "empty-links";
      }
      {
        tool = "pw-link";
        name = "links-empty-verbose-id";
      }
      {
        tool = "pw-link";
        name = "connect-fail";
      }
      {
        tool = "pw-metadata";
        name = "list-empty";
      }
      {
        tool = "pw-metadata";
        name = "connect-fail";
      }
      {
        tool = "pw-dump";
        name = "connect-fail";
      }
      {
        tool = "pw-dump";
        name = "structural";
      }
      {
        tool = "pw-dump";
        name = "structural-indent4";
      }
      {
        tool = "pw-dump";
        name = "structural-indent0";
      }
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
