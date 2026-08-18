{src, ...}: {
  devShells.mojo-wasm = pkgs: let
    inherit (pkgs) lib stdenv;
  in {
    packages = with pkgs;
      [
        just
        mojo
        deno
        wabt
        llvmPackages_latest.llvm
        llvmPackages_latest.lld
        wasmtime.lib
        wasmtime.dev
        jq
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
  checks = pkgs: let
    inherit (pkgs) lib;
    monoSrc = lib.fileset.toSource {
      root = src;
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
        wasm-ld --no-entry --export-all --allow-undefined -mwasm64 \
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
