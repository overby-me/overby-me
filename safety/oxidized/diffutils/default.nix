{
  packages.oxidized-diffutils = {lib, ...}:
    lib.buildCargoProject {
      pname = "rust-diffutils";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };

      index = ../../../platform/nix/lib/cargo/index;

      rootAttrs.postInstall = ''
        ln -s $out/bin/diff $out/bin/cmp
        ln -s $out/bin/diff $out/bin/sdiff
        ln -s $out/bin/diff $out/bin/diff3
      '';

      meta = {
        description = "GNU diffutils-compatible file comparison tools written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/diffutils";
        license = lib.licenses.mit;
        mainProgram = "diff";
        platforms = lib.platforms.linux;
      };
    };
}
