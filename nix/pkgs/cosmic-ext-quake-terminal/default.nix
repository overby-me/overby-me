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
  version = "0.1.0-unstable-2026-02-13";

  src = fetchFromGitHub {
    owner = "M0Rf30";
    repo = "cosmic-ext-quake-terminal";
    rev = "3852de8c453d5c8c6f56130bb9ac1b5a84890c68";
    hash = "sha256-vsS3GATIVXzBiWgdzZfWXUcuV/zuuj5T1u+qntd0/Kg=";
  };

  patches = [./toggle-fix.patch];

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
