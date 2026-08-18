{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  bzip2,
  fontconfig,
  freetype,
  xz,
  zstd,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "datui";
  version = "0.2.49";

  src = fetchFromGitHub {
    owner = "derekwisong";
    repo = "datui";
    tag = "v${finalAttrs.version}";
    hash = "sha256-j9Hk+HkGS6Y2v2kLlusAEUrZLoHs5xHZwViFW7p4cQY=";
  };

  cargoHash = "sha256-eeD0dTRgIkjtUPna2kLwhI9vKqb4mxbBWQj2hEXoIiI=";

  nativeBuildInputs = [
    pkg-config
  ];

  buildInputs = [
    bzip2
    fontconfig
    freetype
    xz
    zstd
  ];

  # Upstream tests have OnceLock poisoning bug in distribution detection
  doCheck = false;

  env = {
    ZSTD_SYS_USE_PKG_CONFIG = true;
  };

  meta = {
    description = "Data Exploration in the Terminal";
    homepage = "https://github.com/derekwisong/datui";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [overby-me];
    mainProgram = "datui";
  };
})
