_: {
  devShells.default = pkgs: let
    inherit (pkgs) lib stdenv;
  in {
    packages = with pkgs;
      [
        just
        mojo
        deno
        wabt
        wasmtime.lib
        wasmtime.dev
        jq
        # test-browser.nu frees its ports with fuser before binding them. It
        # asks inside a try, so an absent fuser reads as "port is free" and the
        # stale server from a previous run survives to answer the next one.
        psmisc
      ]
      # Servo browser engine is broken on Darwin in nixpkgs.
      ++ lib.optionals stdenv.isLinux [
        servo
      ];
  };

  # One check, modeled on dev/mojo/gui's mojo-gui-test: build the WASM binary
  # under -Werror, then compile and run all 52 Mojo test suites through
  # wasmtime. Before this existed the project had no check at all - 83 of its
  # 115 files were verified only transitively, being byte-identical to gui
  # counterparts, and the other 32 only by hand.
  #
  # -Werror is what keeps this tree off deprecated Mojo. The from-source
  # compiler warns where the packaged 26.2.0 did not, and the sources have
  # been migrated to match: pointer subscripts name unsafe_offset, @export'd
  # functions carry an explicit abi("c") effect, allocation goes through
  # std.memory.alloc's unsafe_alloc, and the trait bounds no longer restate
  # what Copyable already implies. The build warns about nothing, so the flag
  # costs nothing until the next deprecation lands - which is when it is
  # worth having.
  checks = pkgs: let
    inherit (pkgs) lib;
    # The repo root as a path literal, not the `src` module argument:
    # `src` is string-like and lib.fileset takes paths only. It must stay
    # the repo root either way, because buildPhase cds into
    # dev/mojo/wasm/web.
    monoSrc = lib.fileset.toSource {
      root = ../../..;
      fileset = lib.fileset.unions [
        ./.
        ../wasmtime/src
      ];
    };
    mojoLinkInputs = with pkgs; [zlib ncurses];
  in {
    mojo-wasm-test = pkgs.stdenv.mkDerivation {
      name = "check-mojo-wasm-test";
      src = monoSrc;

      nativeBuildInputs = with pkgs; [
        just
        mojo
        nushell
        llvmPackages_latest.llvm # llc
        llvmPackages_latest.lld # wasm-ld
        wabt # wasm-objdump etc.
        wasmtime # wasmtime CLI (compile)
      ];

      buildInputs =
        [
          pkgs.wasmtime.lib # libwasmtime.so for test runtime
          pkgs.wasmtime.dev # wasmtime headers / pkg-config
        ]
        ++ mojoLinkInputs;

      buildPhase = ''
        export HOME=$TMPDIR
        export LD_LIBRARY_PATH="${lib.makeLibraryPath [pkgs.wasmtime.lib]}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

        cd dev/mojo/wasm/web
        mkdir -p build
        mojo build -Werror --emit llvm -I ../core/src -I ../examples -I src -o build/out.ll src/main.mojo
        sed -i '/call void @llvm\.lifetime\.\(start\|end\)/d' build/out.ll
        sed -i 's/ nocreateundeforpoison//g' build/out.ll
        sed -i 's/ "target-cpu"="[^"]*"//g; s/ "target-features"="[^"]*"//g' build/out.ll
        sed -i '/^attributes #[0-9]* = { }$/d' build/out.ll
        llc --mtriple=wasm64-wasi -filetype=obj build/out.ll
        # The 8 MiB shadow stack is not optional: html::dsl::_build_node
        # descends a template tree recursively, and on wasm-ld's default
        # stack the descent runs off the bottom, which the runtime reports
        # as "out of bounds memory access" rather than as overflow. Same
        # size dev/mojo/gui links its identical renderer with.
        wasm-ld --no-entry --export-all --allow-undefined -mwasm64 \
          -z stack-size=8388608 \
          --initial-memory=268435456 -o build/out.wasm build/out.o

        wasmtime compile -o build/out.cwasm build/out.wasm

        nu scripts/build-test-binaries.nu
        nu scripts/run-test-binaries.nu
      '';

      installPhase = "touch $out";

      meta.description = "mojo-wasm build (-Werror) + 52 Mojo test suites via wasmtime";
    };
  };
}
