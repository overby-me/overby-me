{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage {
  pname = "wondermagick";
  version = "unstable-2025-09-23";

  src = fetchFromGitHub {
    owner = "Shnatsel";
    repo = "wondermagick";
    rev = "5c479865e4442afa283521b712a70693d358fba9";
    hash = "sha256-sfUcHHo/0BKdHTW10OiOORdnYtuHwmMXUmBCkmsDrEQ=";
  };

  cargoLock = {
    lockFile = ./Cargo.lock;
    outputHashes = {
      "image-0.25.7" = "sha256-NUjB59QjNMSQ8DGz1Yp+u/HVtTtEd9C+ZSZZrDGzaCk=";
    };
  };

  meta = {
    description = "Memory-safe replacement for imagemagick";
    homepage = "https://github.com/Shnatsel/wondermagick";
    license = lib.licenses.unfree; # FIXME: No upstream license https://github.com/Shnatsel/wondermagick/issues/23
    maintainers = with lib.maintainers; [overby-me];
    mainProgram = "wm-convert";
  };
}
