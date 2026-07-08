{
  devShells.wiki-dioxus = pkgs: let
    inherit (pkgs) lib stdenv;
  in {
    packages = with pkgs;
      [
        which
        just
        cargo
        rustc
        rust-analyzer
        dioxus-cli
        wasm-bindgen-cli
        binaryen
        lld
        # Browser testing (test-browser.nu): headless Servo driven over WebDriver.
        curl
        jq
      ]
      # Servo is broken on Darwin in nixpkgs; browser tests are Linux-only.
      ++ lib.optionals stdenv.isLinux [
        servo
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
