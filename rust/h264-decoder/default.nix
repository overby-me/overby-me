{
  devShells.rust-h264-decoder = pkgs: {
    packages = with pkgs; [
      just
    ];
  };

  packages.rust-h264-decoder = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-h264-decoder";
      version = "unstable";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./crates
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      doCheck = false;

      meta = {
        description = "A pure Rust H.264 decoder library";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/h264-decoder";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
        mainProgram = "h264-decode";
      };
    };
}
