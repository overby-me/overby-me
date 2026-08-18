{
  fetchFromGitHub,
  findutils,
  lib,
  pil-squasher,
  stdenv,
}:
stdenv.mkDerivation {
  pname = "firmware-fairphone-fp5";
  # There are no versioned releases, hence the commit hash.
  version = "a4908f548e6f88965e78b1478af1751b6a854fc9";

  src = fetchFromGitHub {
    owner = "FairBlobs";
    repo = "FP5-firmware";
    rev = "a4908f548e6f88965e78b1478af1751b6a854fc9";
    hash = "sha256-XRklo4XfRrskmIxdyY9duU8nF0svoQV90KwaF15ISjk=";
  };

  meta = {
    description = "Firmware files for Fairphone 5";
    longDescription = ''
      Proprietary firmware files required for Fairphone 5 hardware components
      including GPU, DSP, modem, and Bluetooth. Converted from Qualcomm split
      format to monolithic .mbn files for mainline Linux kernel.
    '';
    homepage = "https://github.com/FairBlobs/FP5-firmware";
    license = lib.licenses.unfree;
    maintainers = [];
    platforms = lib.platforms.linux;
  };

  # pil-squasher converts the firmware on the build machine, so it belongs in
  # nativeBuildInputs for cross-compilation to work.
  nativeBuildInputs = [pil-squasher findutils];

  buildPhase = ''
    runHook preBuild

    echo "Squashing firmware files..."
    find . -name "*.mdt" -type f | while read -r mdtfile; do
      echo "Processing: $mdtfile"
      pil-squasher "''${mdtfile%.mdt}.mbn" "$mdtfile"
    done

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    # GPU, DSP and modem firmware.
    mkdir -p "$out/lib/firmware/qcom/qcm6490/fairphone5"
    install -Dm644 -t "$out/lib/firmware/qcom/qcm6490/fairphone5" \
      a660_zap.mbn \
      adsp.mbn \
      cdsp.mbn \
      modem.mbn \
      wpss.mbn

    install -Dm644 -t "$out/lib/firmware/qcom/qcm6490/fairphone5" \
      adspr.jsn \
      adsps.jsn \
      adspua.jsn \
      battmgr.jsn \
      cdspr.jsn \
      modemr.jsn

    # Renamed: the kernel looks for ipa_fws.mbn.
    install -Dm644 yupik_ipa_fws.mbn \
      "$out/lib/firmware/qcom/qcm6490/fairphone5/ipa_fws.mbn"

    # Renamed: the kernel looks for venus.mbn.
    install -Dm644 vpu20_1v.mbn \
      "$out/lib/firmware/qcom/qcm6490/fairphone5/venus.mbn"

    # The speaker amplifier firmware, renamed: the repo calls it aw882xx_acf.bin
    # but the DTS firmware-name property points the driver at aw88261_acf.bin.
    install -Dm644 aw882xx_acf.bin \
      "$out/lib/firmware/qcom/qcm6490/fairphone5/aw88261_acf.bin"

    # Bluetooth.
    mkdir -p "$out/lib/firmware/qca"
    install -Dm644 -t "$out/lib/firmware/qca" \
      msbtfw11.mbn \
      msnv11.bin

    mkdir -p "$out/lib/firmware/qcom/qcm6490/fairphone5"
    cp -r modem_pr "$out/lib/firmware/qcom/qcm6490/fairphone5/"

    find "$out/lib/firmware/qcom/qcm6490/fairphone5/modem_pr" -type f -exec chmod 0644 {} \;

    # HexagonFS, sensors and socinfo only: acdb/ and dsp/ stay out.
    mkdir -p "$out/usr/share/qcom/qcm6490/Fairphone/fp5"
    cp -r hexagonfs/sensors "$out/usr/share/qcom/qcm6490/Fairphone/fp5/"
    cp -r hexagonfs/socinfo "$out/usr/share/qcom/qcm6490/Fairphone/fp5/"

    find "$out/usr/share/qcom/qcm6490/Fairphone/fp5" -type f -exec chmod 0644 {} \;

    runHook postInstall
  '';
}
