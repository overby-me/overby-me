{
  devShells.rust-pkg-config = pkgs: {
    packages = with pkgs; [
      just
    ];
  };

  packages.rust-pkg-config = {lib, ...}:
    lib.buildCargoProject {
      pname = "rust-pkg-config";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./crates
          ./tests
        ];
      };

      index = ../../nix/lib/cargo/index;

      rootAttrs.setupHook = ./setup-hook.sh;

      rootAttrs.postInstall = ''
        ln -s $out/bin/pkgconf $out/bin/pkg-config
      '';

      meta = {
        description = "A pure Rust rewrite and drop-in replacement for pkg-config/pkgconf";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/pkg-config";
        license = lib.licenses.isc;
        maintainers = with lib.maintainers; [overby-me];
        mainProgram = "pkgconf";
        platforms = lib.platforms.linux;
      };
    };
}
