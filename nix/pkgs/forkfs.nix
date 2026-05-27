{
  lib,
  rust-bin,
  makeRustPlatform,
  fetchFromGitHub,
}: let
  rustPlatform = makeRustPlatform {
    cargo = rust-bin.nightly.latest.default;
    rustc = rust-bin.nightly.latest.default;
  };
in
  rustPlatform.buildRustPackage rec {
    pname = "forkfs";
    version = "0.2.8";

    src = fetchFromGitHub {
      owner = "SUPERCILEX";
      repo = "forkfs";
      rev = version;
      hash = "sha256-WrJdk/M40xxzQygP9M1PaMG4jHSHw1iI6AXDJLdnFvs=";
    };

    cargoHash = "sha256-Dwzgm42BUiP2VxMVCtke45Ah5TV+Ip+4zK1PNv5B3hU=";

    postPatch = ''
      substituteInPlace Cargo.toml \
        --replace 'edition = "2021"' 'edition = "2024"'
    '';

    doCheck = false;

    meta = {
      description = "ForkFS allows you to sandbox a process's changes to your file system";
      homepage = "https://github.com/SUPERCILEX/forkfs";
      license = lib.licenses.asl20;
      maintainers = with lib.maintainers; [overby-me];
      mainProgram = "forkfs";
    };
  }
