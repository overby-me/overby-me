{
  packages.rust-bubblewrap = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-bubblewrap";
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
        description = "A bubblewrap-compatible unprivileged sandboxing tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/bubblewrap";
        license = lib.licenses.mit;
        mainProgram = "bwrap";
        platforms = lib.platforms.linux;
      };
    };
}
