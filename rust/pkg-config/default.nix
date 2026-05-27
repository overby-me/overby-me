{
  devShells.rust-pkg-config = pkgs: {
    packages = with pkgs; [
      just
    ];
  };

  packages.rust-pkg-config = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-pkg-config";
      version = "unstable";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./crates
          ./tests
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      setupHook = ./setup-hook.sh;

      postInstall = ''
        ln -s $out/bin/pkgconf $out/bin/pkg-config
      '';

      meta = {
        description = "A pure Rust rewrite and drop-in replacement for pkg-config/pkgconf";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/pkg-config";
        license = lib.licenses.isc;
        maintainers = with lib.maintainers; [overby-me];
        mainProgram = "pkgconf";
      };
    };
}
