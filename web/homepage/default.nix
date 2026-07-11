{
  imports = [
    ./backend
  ];

  devShells.homepage = pkgs: {
    packages = with pkgs; [
      which
      just
      cargo
      rustc
      rust-analyzer
      dioxus-cli
      wasm-bindgen-cli
      binaryen
      lld
    ];
  };

  # Static site build consumed by the host (statichost.eu). Mirrors the
  # wiki-dioxus-frontend package: dx drives cargo/wasm-bindgen/wasm-opt, deps
  # are vendored from Cargo.lock, and the toolchain (dioxus-cli 0.7.x,
  # wasm-bindgen-cli 0.2.x, binaryen) comes straight from nixpkgs.
  packages.homepage = {
    lib,
    rustPlatform,
    dioxus-cli,
    wasm-bindgen-cli,
    binaryen,
    lld,
    ...
  }:
    rustPlatform.buildRustPackage {
      pname = "homepage";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./Dioxus.toml
          ./src
          ./assets
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      nativeBuildInputs = [
        dioxus-cli
        wasm-bindgen-cli
        binaryen
        # dx builds the wasm with lld as the linker.
        lld
      ];

      buildPhase = ''
        runHook preBuild
        dx build --release --platform web
        runHook postBuild
      '';

      # dx 0.7 writes the web bundle here (name from Dioxus.toml).
      installPhase = ''
        runHook preInstall
        cp -r target/dx/homepage/release/web/public $out
        # dx drops files from assets/ it doesn't recognize, so the host's
        # `_redirects` (SPA fallback `/* /index.html 200` + matrix well-knowns)
        # never reaches the bundle. Copy it to the served root ourselves.
        cp assets/_redirects $out/_redirects
        runHook postInstall
      '';

      doCheck = false;

      meta.description = "overby.me homepage — interactive 3D graph (Dioxus + Rust/WASM)";
    };
}
