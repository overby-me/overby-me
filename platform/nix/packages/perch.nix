{
  lib,
  fetchFromGitHub,
  pkg-config,
  sqlite,
  rustPlatform,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "perch";
  version = "0.3.4";

  src = fetchFromGitHub {
    owner = "ricardodantas";
    repo = "perch";
    tag = "v${finalAttrs.version}";
    hash = "sha256-TURpPI4Nj9xfTUEY90KCDgrGFjXGm+/n3cVxxOM709k=";
  };

  cargoHash = "sha256-pbVDG8Wm2K7Hhciq+6xYWbJWYU5CdrAqMXY2ZM1fgfs=";

  nativeBuildInputs = [
    pkg-config
  ];

  buildInputs = [
    sqlite
  ];

  meta = {
    description = "A beautiful terminal social client for Mastodon and Bluesky";
    homepage = "https://github.com/ricardodantas/perch";
    license = lib.licenses.gpl3Only;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
    mainProgram = "perch";
  };
})
