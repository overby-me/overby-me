{
  package = {lib, ...}:
    lib.buildCargoProject {
      pname = "oxidized-bubblewrap";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };

      meta = {
        description = "A bubblewrap-compatible unprivileged sandboxing tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/bubblewrap";
        license = lib.licenses.mit;
        mainProgram = "bwrap";
        platforms = lib.platforms.linux;
      };
    };
}
