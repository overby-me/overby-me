{
  packages.rust-awk = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-awk";
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
        ln -s $out/bin/awk $out/bin/gawk
      '';

      meta = {
        description = "A GNU awk-compatible text processing tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/awk";
        license = lib.licenses.mit;
        mainProgram = "awk";
      };
    };
}
