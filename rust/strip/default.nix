{
  packages.rust-strip = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-strip";
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
        description = "A GNU strip-compatible symbol stripping tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/strip";
        license = lib.licenses.mit;
        mainProgram = "strip";
      };
    };
}
