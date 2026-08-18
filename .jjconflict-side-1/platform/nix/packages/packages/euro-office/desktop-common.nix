# The Euro-Office editors JS/WASM payload, fetched from the image their own CI
# builds and publishes at ghcr.io/euro-office/desktop-common rather than
# rebuilt from the Node20/grunt + Emscripten toolchain. A genuine Euro-Office
# artifact, not an ONLYOFFICE one. Phase 5, approach B; see ./PLAN.md.
#
# Pinned to a super-repo commit (`0bd0e7a`) that has a published image; this
# tracks the submodule revs in ./sources.nix closely. `desktop-common` content
# is architecture-independent (HTML/JS/WASM), so the amd64 image is fine on any
# platform.
{
  lib,
  stdenvNoCC,
  dockerTools,
  python3,
}: let
  sources = import ./sources.nix {inherit lib;};

  # amd64 manifest of ghcr.io/euro-office/desktop-common:0bd0e7a
  # (resolved from the multi-arch index; content is arch-independent).
  payloadImage = dockerTools.pullImage {
    imageName = "ghcr.io/euro-office/desktop-common";
    imageDigest = "sha256:dc8a8303ceb7d2536b83c7ed7810823ccd0a574b6b48972ff8c2d7cc99684955";
    # Output hash of the assembled image tarball.
    sha256 = "sha256-ufoaZUfXCmvU54k+fZs5jB/pDtHQFTuHHYMCTD+nMTs=";
    finalImageName = "euro-office-desktop-common";
    finalImageTag = "0bd0e7a";
    os = "linux";
    arch = "amd64";
  };
in
  stdenvNoCC.mkDerivation {
    pname = "euro-office-desktop-common";
    inherit (sources) version;

    nativeBuildInputs = [python3];

    src = payloadImage;
    dontUnpack = true;

    # The pulled image is a Docker `save` tarball (manifest.json + layer tars).
    # Extract every layer's filesystem into $out, in manifest order.
    installPhase = ''
            runHook preInstall

            mkdir -p image "$out"
            tar -xf "$src" -C image

            for layer in $(python3 -c '
      import json
      with open("image/manifest.json") as f:
          print("\n".join(json.load(f)[0]["Layers"]))
      '); do
              tar -xf "image/$layer" -C "$out" \
                --exclude="dev/*" --exclude="./dev/*" \
                --exclude="etc/*" --exclude="./etc/*" 2>/dev/null || true
            done

            runHook postInstall
    '';

    meta = {
      description = "Euro-Office editors web/WASM payload (official desktop-common build)";
      homepage = "https://github.com/Euro-Office";
      license = lib.licenses.agpl3Plus;
      sourceProvenance = with lib.sourceTypes; [binaryBytecode];
      maintainers = with lib.maintainers; [overby-me];
      platforms = lib.platforms.all;
    };
  }
