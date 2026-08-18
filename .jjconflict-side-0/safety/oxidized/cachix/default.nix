{
  packages.oxidized-cachix = {
    lib,
    pkg-config,
    xz,
    ...
  }:
    lib.buildCargoProject {
      pname = "rust-cachix";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };

      index = ../../../platform/nix/lib/lib/cargo/index;

      crateOverrides.lzma-sys = {
        nativeBuildInputs = [pkg-config];
        buildInputs = [xz];
      };

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
