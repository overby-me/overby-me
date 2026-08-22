{
  packages.default = {lib, ...}:
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

      meta = {
        description = "A POSIX yacc/bison-compatible parser generator written in Rust";
        license = lib.licenses.mit;
        mainProgram = "bison";
        platforms = lib.platforms.linux;
      };
    };
}
