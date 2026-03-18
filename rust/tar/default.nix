{
  packages.rust-tar = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-tar";
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
        description = "A GNU tar-compatible archive tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/tar";
        license = lib.licenses.mit;
        mainProgram = "tar";
      };
    };
}
