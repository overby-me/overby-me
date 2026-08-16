{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage rec {
  pname = "busd";
  version = "0.4.0";

  src = fetchFromGitHub {
    owner = "dbus2";
    repo = "busd";
    rev = version;
    hash = "sha256-y603js+NxqD+SqPA3W+SNCX93exgcEM5g8yZb0wwtw8=";
  };

  cargoHash = "sha256-61gmyFTyS2644TbD9goCgcDDVJpvPKhwDXU/mwNW+60=";

  meta = {
    description = "A D-Bus bus (broker) implementation based on zbus";
    homepage = "https://github.com/dbus2/busd";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
    mainProgram = "busd";
  };
}
