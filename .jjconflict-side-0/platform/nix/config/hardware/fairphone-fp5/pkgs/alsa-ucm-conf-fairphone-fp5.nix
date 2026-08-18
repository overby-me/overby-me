# ALSA UCM (Use Case Manager) configuration for Qualcomm SC7280 devices,
# including the Fairphone 5.  These profiles tell PipeWire / ALSA how to
# route audio to the AW88261 speaker amplifier and WCD9385 codec.
#
# Source: https://github.com/sc7280-mainline/alsa-ucm-conf
# Referenced by: https://gitlab.postmarketos.org/postmarketOS/pmaports
#   device/testing/alsa-ucm-conf-qcom-sc7280/APKBUILD
{
  fetchFromGitHub,
  lib,
  stdenv,
}:
stdenv.mkDerivation {
  pname = "alsa-ucm-conf-fairphone-fp5";
  version = "3-unstable-2025-03-09";

  src = fetchFromGitHub {
    owner = "sc7280-mainline";
    repo = "alsa-ucm-conf";
    rev = "9d5563e6456e1a35e2d59c59130c50b2bbfe3c94";
    hash = "sha256-8OOOzG354x/qmLwQv91C/RrQdZ2L1OyI3Q27/bgmoi0=";
  };

  meta = {
    description = "ALSA UCM configuration for Qualcomm SC7280 / Fairphone 5";
    longDescription = ''
      Use Case Manager profiles that define audio routing for the Fairphone 5
      (QCM6490 / SC7280).  Covers the built-in speakers (AW88261 via quinary
      I2S), bottom microphone (WCD9385 via SoundWire), and DisplayPort audio.

      These profiles replace the stock alsa-ucm-conf for the sound card
      registered as "Fairphone 5" by the sm8250 ASoC machine driver.
    '';
    homepage = "https://github.com/sc7280-mainline/alsa-ucm-conf";
    license = lib.licenses.bsd3;
    maintainers = [];
    platforms = lib.platforms.linux;
  };

  dontBuild = true;

  installPhase = ''
    runHook preInstall

    mkdir -p "$out/share/alsa"
    cp -r ucm2 "$out/share/alsa/"

    runHook postInstall
  '';
}
