_: {
  devShells.mojo-wasm = pkgs: let
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

  # One check, modeled on dev/mojo/gui's mojo-gui-test: build the WASM binary,
  # then compile and run all 52 Mojo test suites through wasmtime. Before this
  # existed the project had no check at all - 83 of its 115 files were verified
  # only transitively, being byte-identical to gui counterparts, and the other
  # 32 only by hand.
  #
  # The build was a -Werror build until mojo started being built from source.
  # That compiler warns where the packaged 26.2.0 did not, and this tree trips
  # 2310 of those warnings in six classes:
  #
  #     1432  positional `__getitem__`      p[0]  ->  p[unsafe_offset=0]
  #      792  @export without an abi effect  def f() -> T:  ->  def f() abi("c") -> T:
  #       43  deprecated symbols
  #       32  `alloc` without a `Layout`
  #       11  redundant trait composition
  #
  # Every one is a deprecation, not a defect: the same sources compile clean
  # without -Werror, and the test suites below still run and still gate. The
  # flag comes back with the migration, which is its own change - these are
  # 2310 edits to hand-written Mojo, and none of them belong in a commit about
  # something else.
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
        mojo build --emit llvm -I ../core/src -I ../examples -I src -o build/out.ll src/main.mojo
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

      meta.description = "mojo-wasm build + 52 Mojo test suites via wasmtime";
    };
  };
}
