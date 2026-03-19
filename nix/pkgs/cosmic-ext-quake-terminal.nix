{
  lib,
  stdenv,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  just,
  libcosmicAppHook,
  libinput,
}:
rustPlatform.buildRustPackage {
  pname = "cosmic-ext-quake-terminal";
  version = "0.1.0";

  src = fetchFromGitHub {
    owner = "M0Rf30";
    repo = "cosmic-ext-quake-terminal";
    tag = "0.1.0";
    hash = "sha256-OzCaepqGczy0a+j2QEODyNnxLgKqoR0GMX3TkBUAh6o=";
  };

  cargoHash = "sha256-846q7q1Rt2z5qkGK1+IHazzuvR8i8IeU0vK/cWn2vwI=";

  nativeBuildInputs = [
    just
    pkg-config
    libcosmicAppHook
  ];

  buildInputs = [
    libinput
  ];

  dontUseJustBuild = true;
  dontUseJustCheck = true;

  justFlags = [
    "--set"
    "prefix"
    (placeholder "out")
    "--set"
    "cargo-target-dir"
    "target/${stdenv.hostPlatform.rust.cargoShortTarget}"
  ];

  meta = {
    homepage = "https://github.com/M0Rf30/cosmic-ext-quake-terminal";
    description = "Quake-style dropdown terminal for COSMIC Desktop";
    license = lib.licenses.gpl3Only;
    platforms = lib.platforms.linux;
    mainProgram = "cosmic-ext-quake-terminal";
  };
}
