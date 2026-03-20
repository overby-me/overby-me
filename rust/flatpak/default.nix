{
  packages.rust-flatpak = {
    lib,
    rustPlatform,
    rust-bubblewrap,
    makeWrapper,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-flatpak";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      nativeBuildInputs = [makeWrapper];

      postInstall = ''
        wrapProgram $out/bin/flatpak \
          --prefix PATH : ${lib.makeBinPath [rust-bubblewrap]}
      '';

      meta = {
        description = "A Flatpak-compatible application sandboxing and distribution tool written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/flatpak";
        license = lib.licenses.mit;
        mainProgram = "flatpak";
        platforms = lib.platforms.linux;
      };
    };
}
