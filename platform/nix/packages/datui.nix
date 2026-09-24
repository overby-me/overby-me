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
  version = "0.3.2";

  src = fetchFromGitHub {
    owner = "derekwisong";
    repo = "datui";
    tag = "v${finalAttrs.version}";
    hash = "sha256-XsasgKdMcahk2i9QHVC9xioE52bLnUX7njtGfhi2rDw=";
  };

  cargoHash = "sha256-0nH7VDGF7/e4HYM+V9+0zZ5/kWQsIC3M903jBy3vmBI=";

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
