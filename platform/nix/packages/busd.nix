{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage rec {
  pname = "busd";
  version = "0.5.0";

  src = fetchFromGitHub {
    owner = "dbus2";
    repo = "busd";
    rev = version;
    hash = "sha256-oEmeFD5UfBq06KmQPOfd2IToD4yF54574Q1L7usJTg0=";
  };

  cargoHash = "sha256-m/ZffZYHu536fjoJknU5P6AFawzFyQ4V7nDYU2kz2ss=";

  meta = {
    description = "A D-Bus bus (broker) implementation based on zbus";
    homepage = "https://github.com/dbus2/busd";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
    mainProgram = "busd";
  };
}
