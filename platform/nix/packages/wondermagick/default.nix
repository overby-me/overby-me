{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage {
  pname = "wondermagick";
  version = "unstable-2026-06-19";

  src = fetchFromGitHub {
    owner = "Shnatsel";
    repo = "wondermagick";
    rev = "dfdf8a91b58269a5813a915e976a960d6815e0c5";
    hash = "sha256-2aupa8J5X0toyXbiU+1t8CT7O/tRFKhnwjAKvOaMUTU=";
  };

  cargoLock = {
    lockFile = ./Cargo.lock;
    outputHashes = {
      "image-0.25.9" = "sha256-7/ETMkDGaFQpFyhRvhAQuHovRg3ig7JdBW3zQM25RUs=";
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
