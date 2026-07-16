{
  imports = [
    ./backend
    ./crates/appview
  ];

  devShells.wiki = pkgs: let
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

  packages.wiki-frontend = {
    lib,
    rustPlatform,
    just,
    which,
    dioxus-cli,
    wasm-bindgen-cli,
    binaryen,
    lld,
    ...
  }:
    rustPlatform.buildRustPackage {
      pname = "wiki-frontend";
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
        # `just build` drives the build; `which` resolves the dioxus `dx` binary
        # the way the justfile expects.
        just
        which
        dioxus-cli
        wasm-bindgen-cli
        binaryen
        # dx links the wasm with lld.
        lld
      ];

      # Run the same `just build` as a local build, so the SPA `_redirects` +
      # root-scoped `sw.js` copies have a single source of truth (the justfile).
      buildPhase = ''
        runHook preBuild
        just build
        runHook postBuild
      '';

      installPhase = ''
        runHook preInstall
        cp -r target/dx/wiki-dioxus/release/web/public $out
        runHook postInstall
      '';

      # Unit tests run separately (`just test`); the nix build only produces the
      # web bundle, and the test fixtures aren't in this package's source set.
      doCheck = false;

      meta.description = "RadikalWiki frontend built with Dioxus + Rust/WASM";
    };
}
