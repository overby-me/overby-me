{
  devShells.default = pkgs: {
    packages = with pkgs; [
      which
      just
      cargo
      rustc
      rust-analyzer
      clippy
      dioxus-cli
      wasm-bindgen-cli
      binaryen
      lld
      # `just browser` drives a headless chromium over the DevTools protocol,
      # serving the bundle from an embedded Deno server.
      deno
    ];
  };

  # The static bundle. Mirrors the homepage package: dx drives
  # cargo/wasm-bindgen/wasm-opt, deps are vendored from Cargo.lock, and the
  # toolchain comes straight from nixpkgs.
  packages.default = {
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
      pname = "randie";
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
          # The firmware and the world it is flown in are workspace members.
          ./firmware
          ./sim
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      nativeBuildInputs = [
        # `just build` drives the build; `which` resolves the dioxus `dx`
        # binary the way the justfile expects.
        just
        which
        dioxus-cli
        wasm-bindgen-cli
        binaryen
        # dx builds the wasm with lld as the linker.
        lld
      ];

      buildPhase = ''
        runHook preBuild
        just build
        runHook postBuild
      '';

      # dx 0.7 writes the web bundle here (name from Dioxus.toml).
      installPhase = ''
        runHook preInstall
        cp -r target/dx/randie/release/web/public $out
        runHook postInstall
      '';

      # The firmware and simulator tests need no browser and no wasm, so they
      # are worth running as part of the build.
      checkPhase = ''
        runHook preCheck
        cargo test --workspace --release
        runHook postCheck
      '';

      meta.description = "Randsim: indoor navigation drone simulator (Dioxus + Rust/WASM)";
    };
}
