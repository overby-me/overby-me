{
  packages.rust-diffutils = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-diffutils";
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
        ln -s $out/bin/diff $out/bin/cmp
        ln -s $out/bin/diff $out/bin/sdiff
        ln -s $out/bin/diff $out/bin/diff3
      '';

      meta = {
        description = "GNU diffutils-compatible file comparison tools written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/diffutils";
        license = lib.licenses.mit;
        mainProgram = "diff";
      };
    };
}
