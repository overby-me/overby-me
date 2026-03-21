{
  packages.rust-binutils = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-binutils";
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
        # Create symlinks for all binutils tools (multicall binary)
        for tool in ar ranlib nm objdump readelf objcopy strings size addr2line c++filt strip as ld; do
          ln -s $out/bin/rust-binutils $out/bin/$tool
        done
      '';

      meta = {
        description = "GNU binutils-compatible binary utilities written in Rust";
        license = lib.licenses.mit;
        mainProgram = "ar";
      };
    };
}
