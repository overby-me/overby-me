{
  devShells.wiki-dioxus = pkgs: {
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

  packages.wiki-dioxus-frontend = {
    lib,
    rustPlatform,
    dioxus-cli,
    wasm-bindgen-cli,
    binaryen,
    ...
  }:
    rustPlatform.buildRustPackage {
      pname = "wiki-dioxus-frontend";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./Dioxus.toml
          ./src
          ./assets
          ./graphql
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      nativeBuildInputs = [
        dioxus-cli
        wasm-bindgen-cli
        binaryen
      ];

      buildPhase = ''
        dx build --release
      '';

      installPhase = ''
        cp -r dist $out
      '';

      meta.description = "RadikalWiki frontend built with Dioxus + Rust/WASM";
    };
}
