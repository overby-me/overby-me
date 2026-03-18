{
  packages.rust-grep = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-grep";
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
        ln -s $out/bin/grep $out/bin/egrep
        ln -s $out/bin/grep $out/bin/fgrep
      '';

      meta = {
        description = "A GNU grep-compatible pattern matching tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/grep";
        license = lib.licenses.mit;
        mainProgram = "grep";
      };
    };
}
