{
  packages.rust-bash = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-bash";
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

      postInstall = ''
        ln -s $out/bin/bash $out/bin/sh
      '';

      meta = {
        description = "A Bash-compatible shell written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/bash";
        license = lib.licenses.mit;
        mainProgram = "bash";
      };
    };
}
