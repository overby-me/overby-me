{
  packages.rust-patchelf = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-patchelf";
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
        description = "A patchelf-compatible ELF binary patching tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/patchelf";
        license = lib.licenses.mit;
        mainProgram = "patchelf";
      };
    };
}
