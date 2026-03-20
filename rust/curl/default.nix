{
  packages.rust-curl = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-curl";
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
        description = "A curl-compatible HTTP client written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/curl";
        license = lib.licenses.mit;
        mainProgram = "curl";
      };
    };
}
