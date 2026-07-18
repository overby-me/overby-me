{
  packages.rust-help2man = {lib, ...}:
    lib.buildCargoProject {
      pname = "rust-help2man";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };

      index = ../../nix/lib/cargo/index;

      meta = {
        description = "A GNU help2man-compatible man page generator written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/help2man";
        license = lib.licenses.mit;
        mainProgram = "help2man";
        platforms = lib.platforms.linux;
      };
    };
}
