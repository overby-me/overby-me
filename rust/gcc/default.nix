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

        # Install compiler built-in headers in GCC-standard location
        # The cc-wrapper uses -B and -isystem to find these
        mkdir -p $out/lib/gcc/x86_64-unknown-linux-gnu/14.2.0/include
        cp -r ${./include}/* $out/lib/gcc/x86_64-unknown-linux-gnu/14.2.0/include/

        # Also create a lib output for the cc-wrapper to reference
        mkdir -p $out/lib
      '';

      meta = {
        description = "A GCC-compatible C compiler written in Rust";
        license = lib.licenses.cc0;
        mainProgram = "gcc";
      };
    };
}
