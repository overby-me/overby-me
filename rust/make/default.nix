{
  packages.rust-make = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-make";
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
        ln -s $out/bin/make $out/bin/gmake
      '';

      meta = {
        description = "A GNU Make-compatible build system driver written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/make";
        license = lib.licenses.mit;
        mainProgram = "make";
      };
    };
}
