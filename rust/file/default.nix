{
  packages.rust-file = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-file";
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
        description = "A GNU file-compatible file type detection tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/file";
        license = lib.licenses.mit;
        mainProgram = "file";
      };
    };
}
