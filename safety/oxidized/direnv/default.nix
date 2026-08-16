{
  packages.oxidized-direnv = {lib, ...}:
    lib.buildCargoProject {
      pname = "rust-direnv";

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
        description = "A Rust rewrite of direnv - unclutter your .profile";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/direnv";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
        mainProgram = "direnv";
        platforms = lib.platforms.linux;
      };
    };
}
