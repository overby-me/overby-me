# Proprietary firmware for the Surface Pro 11 (Qualcomm X1E80100 "Denali").
#
# linux-firmware does not carry it: as of 20260622 the tree has
# qcom/x1e80100/{ASUSTeK,LENOVO,dell,hp} but no `microsoft` vendor directory
# and no Denali topology file, so the blobs have to come from the Windows
# driver CABs that the WOA-Project mirrors.  Only the pieces mainline Linux
# actually loads are extracted; the CABs contain a great deal more.
#
#   qcdxkmsuc8380.mbn  GPU/display microcode, loaded by drm/msm
#   adsp_dtb.mbn       audio DSP devicetree
#   cdsp_dtb.mbn       compute DSP devicetree
#   qcadsp8380.mbn     audio DSP image, loaded by remoteproc
#   qccdsp8380.mbn     compute DSP image, loaded by remoteproc
#   board.bin          ath12k WCN7850 calibration for this board
#
# The ath12k blob is not from Microsoft at all: linux-firmware ships a
# multi-board `board-2.bin` container, and ath12k fails to find the Surface's
# entry in it, so the matching board is unpacked with Qualcomm's own
# `ath12k-bdencoder` and installed under the single-board `board.bin` name that
# the driver falls back to.
#
# Bumping: WOA-Project publishes newer Denali releases (200.0.46.0 as of
# 2026-03) under the same file names.  Change `version`, point `commit` at a
# revision that has that directory, and refresh the three hashes.  200.0.32.0
# is kept because it is the combination that was known to work.
{
  cabextract,
  fetchurl,
  lib,
  linux-firmware,
  python3,
  rsync,
  stdenv,
}:
stdenv.mkDerivation (finalAttrs: {
  pname = "firmware-surface-pro-11";
  version = "200.0.32.0";
  commit = "30e823d85c1fb4e410a4afdf9cd2285914ee712d";

  qcdx8380 = fetchurl {
    url = "https://raw.githubusercontent.com/WOA-Project/Qualcomm-Reference-Drivers/${finalAttrs.commit}/Surface/8380_DEN/${finalAttrs.version}/qcdx8380.cab";
    hash = "sha256-9IsNoOyNfImh0/q5tizDK1ezNuXFT5CmVImQd710p3U=";
  };

  adsp8380 = fetchurl {
    url = "https://raw.githubusercontent.com/WOA-Project/Qualcomm-Reference-Drivers/${finalAttrs.commit}/Surface/8380_DEN/${finalAttrs.version}/surfacepro_ext_adsp8380.cab";
    hash = "sha256-Mk1eeibnGnSSfIlDiKoXb4PnZ02KpibBI1UyFB27WWA=";
  };

  cdsp8380 = fetchurl {
    url = "https://raw.githubusercontent.com/WOA-Project/Qualcomm-Reference-Drivers/${finalAttrs.commit}/Surface/8380_DEN/${finalAttrs.version}/qcnspmcdm_ext_cdsp8380.cab";
    hash = "sha256-TuS/PPCXvbBakCq7nsRpODHyC2nj90l/V6TIxyL3Cy8=";
  };

  board-2 = "${linux-firmware}/lib/firmware/ath12k/WCN7850/hw2.0/board-2.bin";

  bdencoder = fetchurl {
    url = "https://raw.githubusercontent.com/qca/qca-swiss-army-knife/f2164085920540f4ecbfa0b12959918c601724b6/tools/scripts/ath12k/ath12k-bdencoder";
    hash = "sha256-hzn/GVo7nZmuBpuBsEZUcf928w03cgANq+kaUlGmeYA=";
  };

  nativeBuildInputs = [
    cabextract
    rsync
    python3
  ];

  buildCommand = ''
    mkdir -p $out/lib/firmware/{qcom/x1e80100/microsoft/Denali,ath12k/WCN7850/hw2.0}

    cabextract -F qcdxkmsuc8380.mbn $qcdx8380
    mv qcdxkmsuc8380.mbn $out/lib/firmware/qcom/x1e80100/microsoft/qcdxkmsuc8380.mbn

    cabextract -F adsp_dtbs.elf $adsp8380
    mv adsp_dtbs.elf $out/lib/firmware/qcom/x1e80100/microsoft/Denali/adsp_dtb.mbn

    cabextract -F cdsp_dtbs.elf $cdsp8380
    mv cdsp_dtbs.elf $out/lib/firmware/qcom/x1e80100/microsoft/Denali/cdsp_dtb.mbn

    cabextract -F qcadsp8380.mbn -d $out/lib/firmware/qcom/x1e80100/microsoft/Denali/ $adsp8380
    cabextract -F qccdsp8380.mbn -d $out/lib/firmware/qcom/x1e80100/microsoft/Denali/ $cdsp8380

    python3 ${finalAttrs.bdencoder} --extract ${finalAttrs.board-2}
    mv "bus=pci,vendor=17cb,device=1107,subsystem-vendor=17cb,subsystem-device=3378,qmi-chip-id=2,qmi-board-id=255.bin" \
      $out/lib/firmware/ath12k/WCN7850/hw2.0/board.bin

    find "$out" -exec touch --date=2000-01-01 {} +
  '';

  meta = {
    description = "Firmware files for the Microsoft Surface Pro 11";
    longDescription = ''
      Proprietary firmware required by mainline Linux on the Surface Pro 11
      (Qualcomm Snapdragon X Elite, board name "Denali"): GPU/display
      microcode for drm/msm, audio and compute DSP images and devicetrees for
      remoteproc, and the ath12k WCN7850 board calibration data unpacked from
      linux-firmware's multi-board container.

      Extracted from the Windows driver packages mirrored by the WOA-Project,
      because no Denali firmware has been published to linux-firmware.
    '';
    homepage = "https://github.com/WOA-Project/Qualcomm-Reference-Drivers";
    license = lib.licenses.unfree;
    maintainers = [];
    platforms = ["aarch64-linux"];
  };
})
