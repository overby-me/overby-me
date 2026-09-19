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
  version = "0.1.0-unstable-2026-07-16";

  src = fetchFromGitHub {
    owner = "M0Rf30";
    repo = "cosmic-ext-quake-terminal";
    rev = "62df0232b4b4505e84c8e6a8f4a4cb545aff308e";
    hash = "sha256-UJ65rhpvu+4KcpFTI8f12OdVwBH+Mrl5SaIALmABmcI=";
  };

  patches = [./toggle-fix.patch ./wezterm-class-fix.patch];

  cargoHash = "sha256-ZZBWk6IYiUjmWxnDZ/SeHqOqxzyd5LE/nWtNa6N58eU=";

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
