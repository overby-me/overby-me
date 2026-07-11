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
    just,
    which,
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
          ./justfile
          ./src
          ./assets
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      nativeBuildInputs = [
        # `just build` drives the build; `which` resolves the dioxus `dx` binary
        # the way the justfile expects.
        just
        which
        dioxus-cli
        wasm-bindgen-cli
        binaryen
        # dx builds the wasm with lld as the linker.
        lld
      ];

      # `just build` runs `dx build --release` and copies the host `_redirects`
      # into the bundle (dx drops it), so this package matches a local build.
      buildPhase = ''
        runHook preBuild
        just build
        runHook postBuild
      '';

      # dx 0.7 writes the web bundle here (name from Dioxus.toml).
      installPhase = ''
        runHook preInstall
        cp -r target/dx/homepage/release/web/public $out
        runHook postInstall
      '';

      doCheck = false;

      meta.description = "overby.me homepage — interactive 3D graph (Dioxus + Rust/WASM)";
    };
}
