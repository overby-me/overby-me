let
  # The dx-built web bundle. Mirrors the randie package: dx drives
  # cargo/wasm-bindgen/wasm-opt, deps are vendored from Cargo.lock.
  mkFrontend = {
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
      pname = "webxr-compositor-frontend";
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
          # The wire protocol is a workspace member.
          ./protocol
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      nativeBuildInputs = [
        just
        which
        dioxus-cli
        wasm-bindgen-cli
        binaryen
        # dx links the wasm with lld.
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
        cp -r target/dx/webxr-compositor/release/web/public $out
        runHook postInstall
      '';

      checkPhase = ''
        runHook preCheck
        cargo test --workspace --release
        runHook postCheck
      '';

      meta.description = "webxr-compositor browser frontend (Dioxus + Rust/WASM)";
    };

  # The native host binary, a cargo workspace of its own under host/.
  mkHost = {
    lib,
    rustPlatform,
    pkg-config,
    libxkbcommon,
    ...
  }:
    rustPlatform.buildRustPackage {
      pname = "webxr-compositor-host";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./host
          # Path dependency of the host workspace.
          ./protocol
        ];
      };

      cargoRoot = "host";
      buildAndTestSubdir = "host";
      cargoLock.lockFile = ./host/Cargo.lock;

      nativeBuildInputs = [pkg-config];
      # smithay's seat keyboard state is xkbcommon.
      buildInputs = [libxkbcommon];

      meta.description = "webxr-compositor native host (Wayland socket + HTTP/WebSocket server)";
    };
in {
  devShell = pkgs: {
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
      # Browser testing will drive a headless chromium served by deno, like
      # the sibling apps do.
      deno
      # The host links libxkbcommon through smithay.
      pkg-config
      libxkbcommon
      # test-wayland.nu asks wayland-info what the host advertises.
      wayland-utils
      # test-input.nu types into a real terminal client.
      foot
      # test-gtk.nu drives a real GTK4 app's menus and popovers.
      gnome-calculator
    ];
  };

  packages.webxr-compositor-frontend = mkFrontend;

  # The runnable app: the host wired to the built frontend, so
  # `nix run` serves the real bundle out of the store.
  packages.webxr-compositor = {
    lib,
    rustPlatform,
    dioxus-cli,
    wasm-bindgen-cli,
    binaryen,
    lld,
    just,
    which,
    pkg-config,
    libxkbcommon,
    makeBinaryWrapper,
    symlinkJoin,
    ...
  }: let
    frontend = mkFrontend {
      inherit lib rustPlatform dioxus-cli wasm-bindgen-cli binaryen lld just which;
    };
    host = mkHost {inherit lib rustPlatform pkg-config libxkbcommon;};
  in
    symlinkJoin {
      name = "webxr-compositor";
      paths = [host];
      nativeBuildInputs = [makeBinaryWrapper];
      postBuild = ''
        wrapProgram $out/bin/webxr-compositor \
          --set-default WEBXR_COMPOSITOR_WEB_ROOT ${frontend}
      '';
      meta.description = "webxr-compositor: Wayland apps in the browser";
    };
}
