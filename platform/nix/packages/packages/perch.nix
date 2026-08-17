{
  lib,
  fetchFromGitHub,
  pkg-config,
  sqlite,
  makeRustPlatform,
  rust-bin,
}: let
  rustPlatform = makeRustPlatform {
    cargo = rust-bin.stable.latest.default;
    rustc = rust-bin.stable.latest.default;
  };
in
  rustPlatform.buildRustPackage (finalAttrs: {
    pname = "perch";
    version = "0.3.2";

    src = fetchFromGitHub {
      owner = "ricardodantas";
      repo = "perch";
      tag = "v${finalAttrs.version}";
      hash = "sha256-rpN/q3RL7WCLkcR4DAcm9Vu6QlAN/NO14eFL/3qhAgo=";
    };

    cargoHash = "sha256-HxVZ5YWEPMPLXHJZfIfIbVAdkEzCsK5AJIQnqfZVVSg=";

    nativeBuildInputs = [
      pkg-config
    ];

    buildInputs = [
      sqlite
    ];

    meta = {
      description = "A beautiful terminal social client for Mastodon and Bluesky";
      homepage = "https://github.com/ricardodantas/perch";
      license = lib.licenses.gpl3Only;
      maintainers = with lib.maintainers; [overby-me];
      platforms = lib.platforms.linux;
      mainProgram = "perch";
    };
  })
