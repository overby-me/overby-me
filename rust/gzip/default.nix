{
  packages.rust-gzip = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-gzip";
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
        ln -s $out/bin/gzip $out/bin/gunzip
        ln -s $out/bin/gzip $out/bin/zcat
      '';

      meta = {
        description = "A GNU gzip-compatible compression tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/gzip";
        license = lib.licenses.mit;
        mainProgram = "gzip";
      };
    };
}
