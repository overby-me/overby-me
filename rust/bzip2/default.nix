{
  packages.rust-bzip2 = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-bzip2";
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
        ln -s $out/bin/bzip2 $out/bin/bunzip2
        ln -s $out/bin/bzip2 $out/bin/bzcat
      '';

      meta = {
        description = "A bzip2-compatible compression tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/bzip2";
        license = lib.licenses.mit;
        mainProgram = "bzip2";
      };
    };
}
