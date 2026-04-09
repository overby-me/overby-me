{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "deslop";
  version = "0.2.0";

  src = fetchFromGitHub {
    owner = "chinmay-sawant";
    repo = "deslop";
    tag = "v${finalAttrs.version}";
    hash = "sha256-9k+RyWJPS2/x0bB6o94FPuM0Qp7NpkH2Cn2HgIgTY2Q=";
  };

  cargoHash = "sha256-pcS9kAR443tovTpVNPnJSxrNW7pJhrJ0qODNty7Tbn8=";

  meta = {
    description = "Language Agnostic Ultra Fast Best Practice Analyzer";
    homepage = "https://github.com/chinmay-sawant/deslop";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [noverby];
    mainProgram = "deslop";
  };
})
