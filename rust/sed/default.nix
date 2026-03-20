{
  packages.rust-sed = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-sed";
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
        description = "A GNU sed-compatible stream editor written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/sed";
        license = lib.licenses.mit;
        mainProgram = "sed";
      };
    };
}
