{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  stdenv,
  wayland,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "stochos";
  version = "0.3.2";

  src = fetchFromGitHub {
    owner = "museslabs";
    repo = "stochos";
    tag = "v${finalAttrs.version}";
    hash = "sha256-UCwMsoqBwRXbItmxtKjhfb8Ua0srjizueWa2LORHL/s=";
  };

  cargoHash = "sha256-z+7c2Qat+tX+t4hLeim/GVdbpY6o8hD2rZZP8zahWPc=";

  nativeBuildInputs = [
    pkg-config
  ];

  buildInputs = lib.optionals stdenv.isLinux [
    wayland
  ];

  meta = {
    description = "Keyboard driven mouse control";
    homepage = "https://github.com/museslabs/stochos";
    license = lib.licenses.gpl3Plus;
    maintainers = with lib.maintainers; [noverby];
    mainProgram = "stochos";
  };
})
