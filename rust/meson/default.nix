{
  packages.rust-meson = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-meson";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      meta = {
        description = "A Meson build system compatible implementation in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/meson";
        license = lib.licenses.mit;
        mainProgram = "meson";
      };
    };
}
