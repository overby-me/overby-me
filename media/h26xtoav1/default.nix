{
  devShells.h26xtoav1 = pkgs: {
    packages = with pkgs; [
      just
    ];
  };

  packages.h26xtoav1 = {lib, ...}:
    lib.buildCargoProject {
      pname = "rust-h26xtoav1";

      src = lib.fileset.toSource {
        root = ./..;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ../h264-decoder/Cargo.toml
          ../h264-decoder/Cargo.lock
          ../h264-decoder/crates
          ../h265-decoder/Cargo.toml
          ../h265-decoder/Cargo.lock
          ../h265-decoder/crates
        ];
      };

      manifestDir = "h26xtoav1";

      index = ../../platform/nix/config/lib/cargo/index;

      meta = {
        description = "A CLI tool to transcode H.264/H.265 video to AV1 using h264-decode, h265-decode, and rav1e";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/h26xtoav1";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
        mainProgram = "h26xtoav1";
        platforms = lib.platforms.linux;
      };
    };
}
