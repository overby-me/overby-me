{
  packages.rust-help2man = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-help2man";
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
        description = "A GNU help2man-compatible man page generator written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/help2man";
        license = lib.licenses.mit;
        mainProgram = "help2man";
      };
    };
}
