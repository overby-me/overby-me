{
  packages.rust-direnv = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-direnv";
      version = "2.36.0";

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
        description = "A Rust rewrite of direnv - unclutter your .profile";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/direnv";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [noverby];
        mainProgram = "direnv";
      };
    };
}
