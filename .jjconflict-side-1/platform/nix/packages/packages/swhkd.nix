{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage {
  pname = "swhkd";
  version = "1.2.1";

  src = fetchFromGitHub {
    owner = "waycrate";
    repo = "swhkd";
    rev = "1.2.1";
    hash = "sha256-VQW01j2RxhLUx59LAopZEdA7TyZBsJrF1Ym3LumvFqA=";
  };

  cargoHash = "sha256-RGO9kEttGecllzH0gBIW6FnoSHGcrDfLDf2omUqKxX4=";

  meta = {
    homepage = "https://github.com/waycrate/swhkd";
    description = "Simple Wayland HotKey Daemon — sxhkd clone for Wayland";
    license = lib.licenses.bsd2;
    platforms = lib.platforms.linux;
  };
}
