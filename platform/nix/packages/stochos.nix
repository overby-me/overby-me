{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  stdenv,
  wayland,
  libxkbcommon,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "stochos";
  version = "1.0.1";

  src = fetchFromGitHub {
    owner = "museslabs";
    repo = "stochos";
    tag = "v${finalAttrs.version}";
    hash = "sha256-4EEijphUD1lLjBMdsV5asBuSNJlzTtUkzd3ujzmHOoI=";
  };

  cargoHash = "sha256-98z0x0MovUIA+DZPR5o4rH/Gm2cBsWBvGefcCLyuFjc=";

  nativeBuildInputs = [
    pkg-config
  ];

  buildInputs = lib.optionals stdenv.isLinux [
    wayland
    libxkbcommon
  ];

  meta = {
    description = "Keyboard driven mouse control";
    homepage = "https://github.com/museslabs/stochos";
    license = lib.licenses.gpl3Plus;
    maintainers = with lib.maintainers; [overby-me];
    mainProgram = "stochos";
  };
})
