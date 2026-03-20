{
  packages.rust-texinfo = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-texinfo";
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

      # Provide texi2any as an alias (some build systems use it)
      postInstall = ''
        ln -s $out/bin/makeinfo $out/bin/texi2any
      '';

      meta = {
        description = "A GNU makeinfo-compatible Texinfo processor written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/texinfo";
        license = lib.licenses.mit;
        mainProgram = "makeinfo";
      };
    };
}
