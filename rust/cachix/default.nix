{
  packages.rust-cachix = {
    lib,
    rustPlatform,
    pkg-config,
    openssl,
    xz,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-cachix";
      version = "unstable";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      nativeBuildInputs = [
        pkg-config
      ];

      buildInputs = [
        openssl
        xz
      ];

      meta = {
        description = "Rust port of the Cachix CLI client for Nix binary cache hosting";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/cachix";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
        mainProgram = "cachix";
        platforms = lib.platforms.linux;
      };
    };
}
