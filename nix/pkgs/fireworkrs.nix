{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "fireworkrs";
  version = "0.2.1";

  src = fetchFromGitHub {
    owner = "crisidev";
    repo = "fireworkrs";
    tag = "v${finalAttrs.version}";
    hash = "sha256-rqBxDiNa2H8MLCBS6qzXO255aSSZut2fwVyEOQeaIac=";
  };

  cargoHash = "sha256-4BKgDCH6/PfsAwvk8WrZ+5z3AdAdf5EYbOmf2BHJ2ZY=";

  meta = {
    description = "Play text art animations in your terminal";
    homepage = "https://github.com/crisidev/fireworkrs";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [noverby];
    mainProgram = "fireworkrs";
  };
})
