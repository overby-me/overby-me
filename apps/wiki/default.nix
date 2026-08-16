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
        # Splits the DWARF out of the built wasm into a sidecar, so a crash can
        # be traced to a source line without shipping ~20 MB to every visitor
        # (scripts/split-symbols.nu).
        wasm-tools
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
    nushell,
    wasm-tools,
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
          # The page shell: the boot screen, the wasm-fetch recovery path and
          # the `heicDecode` worker glue. Without it dx writes a default one,
          # and this build quietly stopped matching what `just build` deploys.
          ./index.html
          # The HEIC decoder's own wasm module, built by `just build` into a
          # Web Worker (heic-worker/). A workspace member, so it is vendored
          # from the same Cargo.lock as everything else.
          ./heic-worker
          # Path dependencies: the OOXML parsers and the patched dioxus-core.
          # Cargo cannot resolve the manifest without them.
          ./vendor
          # The scripts `just build` runs, named one by one rather than as the
          # whole scripts/ directory, so editing an unrelated script does not
          # rebuild the frontend. Every script the recipe calls has to be here:
          # inject-wasm-size.nu was added to the recipe without being added
          # here, and this build failed on it from then until it was noticed.
          ./scripts/split-symbols.nu
          ./scripts/inject-wasm-size.nu
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
        # `just build` finishes with scripts/split-symbols.nu, which moves the
        # DWARF out of the shipped wasm into a sidecar. Without these two the
        # hermetic build died there ("nu: command not found"), so the documented
        # `just deploy-build` could not produce a bundle at all.
        nushell
        wasm-tools
      ];

      # Run the same `just build` as a local build, so the SPA `_redirects`,
      # the `_headers` cache policy and the root-scoped `sw.js` copies have a
      # single source of truth (the justfile).
      #
      # The bundle records the commit it was built from (src/build_info.rs), and
      # this build cannot know it: the source set above has no `.git`, and taking
      # the flake's rev would rebuild the frontend on every unrelated commit. So
      # it reports `unknown` unless a caller passes one — `GIT_COMMIT=<rev> nix
      # build …` — which the justfile prefers over its own lookup. Deploys go
      # through `just build` in the devshell (it bakes in the Better Stack token
      # this hermetic build has no access to), and that path resolves the commit
      # properly.
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
