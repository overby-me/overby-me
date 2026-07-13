{
  imports = [
    ./backend
  ];

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
    lld,
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

      cargoLock = {
        lockFile = ./Cargo.lock;
        # The Dioxus component library is pinned to a git rev, so its checkout
        # needs an explicit FOD hash to vendor reproducibly.
        outputHashes = {
          "dioxus-attributes-0.1.0" = "sha256-QjZOGBtnmS+bncNCGpJpuACroUuxy61WA/Sq6P5aUc0=";
          "dioxus-primitives-0.0.1" = "sha256-QjZOGBtnmS+bncNCGpJpuACroUuxy61WA/Sq6P5aUc0=";
        };
      };

      nativeBuildInputs = [
        dioxus-cli
        wasm-bindgen-cli
        binaryen
        # dx links the wasm with lld.
        lld
      ];

      buildPhase = ''
        dx build --release
      '';

      installPhase = ''
        cp -r target/dx/wiki-dioxus/release/web/public $out
        # Serve the service worker from the site ROOT so its scope is `/` (not the
        # hashed `/assets/` path, whose scope is only `/assets/`). statichost.eu
        # serves $out at the domain root, so $out/sw.js is reachable at /sw.js and
        # can control the whole app (/, /wasm/*) for offline use.
        cp ${./assets/sw.js} $out/sw.js
      '';

      # Unit tests run separately (`just test`); the nix build only produces the
      # web bundle, and the test fixtures aren't in this package's source set.
      doCheck = false;

      meta.description = "RadikalWiki frontend built with Dioxus + Rust/WASM";
    };
}
