{
  packages.rust-gcc = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-gcc";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./include
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      doCheck = false;

      postInstall = ''
        # Create standard GCC symlinks
        ln -s $out/bin/gcc $out/bin/cc
        ln -s $out/bin/gcc $out/bin/x86_64-unknown-linux-gnu-gcc

        # Install compiler built-in headers
        mkdir -p $out/lib/gcc/include
        cp -r ${./include}/* $out/lib/gcc/include/
      '';

      meta = {
        description = "A GCC-compatible C compiler written in Rust";
        license = lib.licenses.cc0;
        mainProgram = "gcc";
      };
    };
}
