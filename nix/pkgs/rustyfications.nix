{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  gtk4,
  gtk4-layer-shell,
  systemd,
}:
rustPlatform.buildRustPackage {
  pname = "rustyfications";
  version = "0.1.12-alpha";

  src = fetchFromGitHub {
    owner = "bzglve";
    repo = "rustyfications";
    tag = "v0.1.12-alpha";
    hash = "sha256-QWoSz/1ft3vibK/QdZwNCOOdQtNa7sdzhyL7KKpCREI=";
  };

  cargoHash = "sha256-GndADwqsnGtNAESnboYE1fqQN1CArHY2yTR61x3d3Ig=";

  nativeBuildInputs = [
    pkg-config
  ];

  buildInputs = [
    gtk4
    gtk4-layer-shell
    systemd
  ];

  meta = {
    description = "Rusty notification daemon for Wayland";
    homepage = "https://github.com/bzglve/rustyfications";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
    mainProgram = "rustyfications";
  };
}
