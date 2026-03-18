{
  lib,
  fetchFromGitHub,
  rustPlatform,
}:
rustPlatform.buildRustPackage {
  pname = "uutils-sed";
  version = "0.1.1";

  src = fetchFromGitHub {
    owner = "uutils";
    repo = "sed";
    tag = "0.1.1";
    hash = "sha256-y1X9nj/quBtisp+6MHFjVKFHrdFnujWTxLWNLvdrADA=";
  };

  cargoHash = "sha256-N5wwNPjOL3U4bPSONGpjmOBU31Nt/sCVth+JH3xmz/g=";

  meta = {
    description = "Rewrite of sed in Rust";
    homepage = "https://github.com/uutils/sed";
    license = lib.licenses.mit;
    mainProgram = "sed";
    platforms = lib.platforms.unix;
  };
}
