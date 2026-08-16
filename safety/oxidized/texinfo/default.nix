{
  packages.oxidized-texinfo = {lib, ...}:
    lib.buildCargoProject {
      pname = "rust-texinfo";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };

      index = ../../../platform/nix/lib/cargo/index;

      # Provide texi2any as an alias (some build systems use it)
      rootAttrs.postInstall = ''
        ln -s $out/bin/makeinfo $out/bin/texi2any
      '';

      meta = {
        platforms = lib.platforms.linux;
        description = "A GNU makeinfo-compatible Texinfo processor written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/texinfo";
        license = lib.licenses.mit;
        mainProgram = "makeinfo";
      };
    };
}
