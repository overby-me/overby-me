{
  devShells.default = pkgs: {
    packages = with pkgs; [
      just
    ];
  };

  packages.default = {lib, ...}:
    lib.buildCargoProject {
      pname = "rust-h264-decoder";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./crates
        ];
      };

      meta = {
        description = "A pure Rust H.264 decoder library";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/h264-decoder";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
        mainProgram = "h264-decode";
        platforms = lib.platforms.linux;
      };
    };
}
