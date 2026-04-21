{
  packages = {
    rust-xz = {
      lib,
      rustPlatform,
      xz,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-xz";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
            ./benches
            ./tests
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = [xz];

        postInstall = ''
          ln -s $out/bin/xz $out/bin/unxz
          ln -s $out/bin/xz $out/bin/xzcat
          ln -s $out/bin/xz $out/bin/lzma
          ln -s $out/bin/xz $out/bin/unlzma
          ln -s $out/bin/xz $out/bin/lzcat
        '';

        meta = {
          description = "An xz-compatible LZMA compression tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/xz";
          license = lib.licenses.mit;
          mainProgram = "xz";
        };
      };

    # Debug build used by the test suite for fast turnaround. Same
    # trick `rust-awk-dev` uses.
    rust-xz-dev = {
      lib,
      rustPlatform,
      xz,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-xz-dev";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
            ./benches
            ./tests
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = [xz];

        buildType = "debug";

        postInstall = ''
          ln -s $out/bin/xz $out/bin/unxz
          ln -s $out/bin/xz $out/bin/xzcat
          ln -s $out/bin/xz $out/bin/lzma
          ln -s $out/bin/xz $out/bin/unlzma
          ln -s $out/bin/xz $out/bin/lzcat
        '';

        meta = {
          description = "rust-xz built in debug mode for the test suite";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/xz";
          license = lib.licenses.mit;
          mainProgram = "xz";
        };
      };
  };

  checks = let
    # Filenames inside `xz-5.8.1/tests/files/` of the upstream tarball.
    # Kept here as a static list (no IFD) so `nix flake check` can
    # enumerate every per-file derivation without unpacking the tarball.
    goodFiles = [
      "good-0-empty.xz"
      "good-0cat-empty.xz"
      "good-0catpad-empty.xz"
      "good-0pad-empty.xz"
      "good-1-3delta-lzma2.xz"
      "good-1-arm64-lzma2-1.xz"
      "good-1-arm64-lzma2-2.xz"
      "good-1-block_header-1.xz"
      "good-1-block_header-2.xz"
      "good-1-block_header-3.xz"
      "good-1-check-crc32.xz"
      "good-1-check-crc64.xz"
      "good-1-check-none.xz"
      "good-1-check-sha256.xz"
      "good-1-delta-lzma2.tiff.xz"
      "good-1-empty-bcj-lzma2.xz"
      "good-1-lzma2-1.xz"
      "good-1-lzma2-2.xz"
      "good-1-lzma2-3.xz"
      "good-1-lzma2-4.xz"
      "good-1-lzma2-5.xz"
      "good-1-v0.lz"
      "good-1-v0-trailing-1.lz"
      "good-1-v1.lz"
      "good-1-v1-trailing-1.lz"
      "good-1-v1-trailing-2.lz"
      "good-2-lzma2.xz"
      "good-2-v0-v1.lz"
      "good-2-v1-v0.lz"
      "good-2-v1-v1.lz"
      "good-known_size-with_eopm.lzma"
      "good-known_size-without_eopm.lzma"
      "good-unknown_size-with_eopm.lzma"
    ];

    badFiles = [
      "bad-0-backward_size.xz"
      "bad-0-empty-truncated.xz"
      "bad-0-footer_magic.xz"
      "bad-0-header_magic.xz"
      "bad-0-nonempty_index.xz"
      "bad-0cat-alone.xz"
      "bad-0cat-header_magic.xz"
      "bad-0catpad-empty.xz"
      "bad-0pad-empty.xz"
      "bad-1-block_header-1.xz"
      "bad-1-block_header-2.xz"
      "bad-1-block_header-3.xz"
      "bad-1-block_header-4.xz"
      "bad-1-block_header-5.xz"
      "bad-1-block_header-6.xz"
      "bad-1-check-crc32-2.xz"
      "bad-1-check-crc32.xz"
      "bad-1-check-crc64.xz"
      "bad-1-check-sha256.xz"
      "bad-1-lzma2-1.xz"
      "bad-1-lzma2-2.xz"
      "bad-1-lzma2-3.xz"
      "bad-1-lzma2-4.xz"
      "bad-1-lzma2-5.xz"
      "bad-1-lzma2-6.xz"
      "bad-1-lzma2-7.xz"
      "bad-1-lzma2-8.xz"
      "bad-1-lzma2-9.xz"
      "bad-1-lzma2-10.xz"
      "bad-1-lzma2-11.xz"
      "bad-1-stream_flags-1.xz"
      "bad-1-stream_flags-2.xz"
      "bad-1-stream_flags-3.xz"
      "bad-1-v0-uncomp-size.lz"
      "bad-1-v1-crc32.lz"
      "bad-1-v1-dict-1.lz"
      "bad-1-v1-dict-2.lz"
      "bad-1-v1-magic-1.lz"
      "bad-1-v1-magic-2.lz"
      "bad-1-v1-member-size.lz"
      "bad-1-v1-trailing-magic.lz"
      "bad-1-v1-uncomp-size.lz"
      "bad-1-vli-1.xz"
      "bad-1-vli-2.xz"
      "bad-2-compressed_data_padding.xz"
      "bad-2-index-1.xz"
      "bad-2-index-2.xz"
      "bad-2-index-3.xz"
      "bad-2-index-4.xz"
      "bad-2-index-5.xz"
      "bad-3-index-uncomp-overflow.xz"
      "bad-too_big_size-with_eopm.lzma"
      "bad-too_small_size-without_eopm-1.lzma"
      "bad-too_small_size-without_eopm-2.lzma"
      "bad-too_small_size-without_eopm-3.lzma"
      "bad-unknown_size-without_eopm.lzma"
      "unsupported-block_header.xz"
      "unsupported-filter_flags-1.xz"
      "unsupported-filter_flags-2.xz"
      "unsupported-filter_flags-3.xz"
      "unsupported-1-v234.lz"
      "unsupported-check.xz"
    ];

    # Each entry describes how to invoke one upstream
    # `tests/test_*.sh` script. `allowSkip` is true only when the
    # script's preconditions are out-of-scope for rust-xz; otherwise
    # an exit-77 ("skip") is treated as a failure so we never
    # silently miss coverage.
    upstreamScripts = [
      { name = "test_files"; }
      { name = "test_suffix"; }
      # `test_compress.sh` itself only does anything when called
      # with a generated/prepared input filename, so we drive it via
      # the three `test_compress_generated_*` wrappers below.
      { name = "test_compress_generated_abc"; needsHelper = true;
        compressFile = "compress_generated_abc"; }
      { name = "test_compress_generated_random"; needsHelper = true;
        compressFile = "compress_generated_random"; }
      { name = "test_compress_generated_text"; needsHelper = true;
        compressFile = "compress_generated_text"; }
      # `test_scripts.sh` exercises `xzdiff`/`xzgrep` shell wrappers
      # we don't ship, so it self-skips. Mark allowSkip so the
      # derivation is green without falsely claiming coverage.
      { name = "test_scripts"; allowSkip = true; }
    ];

    # Replace `_` with `-` so attribute names render nicely.
    sanitize = builtins.replaceStrings ["_"] ["-"];

    scriptChecks = builtins.listToAttrs (map (s: {
        name = "rust-xz-${sanitize s.name}";
        value = pkgs: (import ./testsuite.nix {inherit pkgs;}).script s;
      })
      upstreamScripts);

    # Upstream C unit tests for liblzma. These don't exercise the
    # rust-xz CLI directly, but they validate the C library it links
    # against — wiring them gives us full upstream-test parity.
    upstreamCTests = [
      "test_check"
      "test_hardware"
      "test_stream_flags"
      "test_filter_flags"
      "test_filter_str"
      "test_block_header"
      "test_index"
      "test_index_hash"
      "test_bcj_exact_size"
      "test_memlimit"
      "test_lzip_decoder"
      "test_vli"
    ];

    cTestChecks = builtins.listToAttrs (map (n: {
        name = "rust-xz-${sanitize n}";
        value = pkgs: (import ./testsuite.nix {inherit pkgs;}).cTest n;
      })
      upstreamCTests);

    goodChecks = builtins.listToAttrs (map (n: {
        name = "rust-xz-${sanitize n}";
        value = pkgs: (import ./testsuite.nix {inherit pkgs;}).file {
          name = n;
          expect = "good";
        };
      })
      goodFiles);

    badChecks = builtins.listToAttrs (map (n: {
        name = "rust-xz-${sanitize n}";
        value = pkgs: (import ./testsuite.nix {inherit pkgs;}).file {
          name = n;
          expect = "bad";
        };
      })
      badFiles);
  in
    scriptChecks
    // cTestChecks
    // goodChecks
    // badChecks
    // {
      rust-xz-roundtrip = pkgs: (import ./testsuite.nix {inherit pkgs;}).roundtrip;
      rust-xz-list = pkgs: (import ./testsuite.nix {inherit pkgs;}).list;
      rust-xz-filters = pkgs: (import ./testsuite.nix {inherit pkgs;}).filters;
      rust-xz-fuzz = pkgs: (import ./testsuite.nix {inherit pkgs;}).fuzz;
    };
}
