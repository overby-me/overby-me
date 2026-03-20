{
  packages.rust-xz = {
    lib,
    rustPlatform,
    xz,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-xz";
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

      nativeBuildInputs = [xz];

      postInstall = ''
        ln -s $out/bin/xz $out/bin/unxz
        ln -s $out/bin/xz $out/bin/xzcat
        ln -s $out/bin/xz $out/bin/lzma
        ln -s $out/bin/xz $out/bin/unlzma
        ln -s $out/bin/xz $out/bin/lzcat
      '';

      meta = {
        description = "An xz-compatible LZMA compression tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/xz";
        license = lib.licenses.mit;
        mainProgram = "xz";
      };
    };
}
