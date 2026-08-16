{
  packages.oxidized-bison = {lib, ...}:
    lib.buildCargoProject {
      pname = "rust-bison";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };

      index = ../../../platform/nix/lib/cargo/index;

      meta = {
        description = "A POSIX yacc/bison-compatible parser generator written in Rust";
        license = lib.licenses.mit;
        mainProgram = "bison";
        platforms = lib.platforms.linux;
      };
    };
}
