{
  devShell = pkgs: {
    packages = with pkgs; [
      just
    ];
  };

  package = {lib, ...}:
    lib.buildCargoProject {
      pname = "oxidized-pkg-config";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./crates
          ./tests
        ];
      };

      # This workspace has two root crates (libpkgconf + pkgconf), so
      # buildCargoProject wraps them in a symlinkJoin, which runs no stdenv
      # phases: setupHook and postInstall are silently ignored there.
      # postBuild is the only hook symlinkJoin executes, so install both
      # the pkg-config compatibility symlink and the setup hook from it.
      rootAttrs.postBuild = ''
        ln -s $out/bin/pkgconf $out/bin/pkg-config
        mkdir -p $out/nix-support
        cp ${./setup-hook.sh} $out/nix-support/setup-hook
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
