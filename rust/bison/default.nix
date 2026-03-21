{
  packages.rust-bison = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-bison";
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
        description = "A POSIX yacc/bison-compatible parser generator written in Rust";
        license = lib.licenses.mit;
        mainProgram = "bison";
      };
    };
}
