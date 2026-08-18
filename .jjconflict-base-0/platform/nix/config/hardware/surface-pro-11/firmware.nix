# Surface Pro 11 firmware, from two sources.
#
# linux-firmware, always: the Adreno X1-85 needs gen70500_sqe.fw,
# gen70500_gmu.bin and x1e80100/gen70500_zap.mbn. The zap shader is the one
# that bites, being what moves the GPU out of its secure boot state: without it
# the GPU's SMMU probe times out (-110), msm_dpu never binds, simpledrm keeps
# /dev/dri/card0 with no render node, and the first compositor to start spins
# on it at 100% CPU against a black screen. Redistributable, so not gated
# behind the option below.
#
# The Microsoft blobs, opt-in: unfree, fetched from a mirror of the Windows
# driver packages, and covering the DSPs, display microcode and Wi-Fi board
# data.  Opting in is a deliberate act.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.hardware.surfacePro11;
in {
  imports = [
    (lib.mkRemovedOptionModule
      ["hardware" "surfacePro11" "firmware" "path"]
      "The firmware is now downloaded and extracted automatically.")
  ];

  options.hardware.surfacePro11.firmware = {
    enable = lib.mkEnableOption "the firmware needed for the Surface Pro 11";
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    {
      # 1.7 GiB of linux-firmware for three files is poor value, but the
      # alternative is copying them out by hand and rediscovering the next
      # missing one the same slow way.
      hardware.enableRedistributableFirmware = true;

      # Qualcomm remoteproc images load with request_firmware_into_buf, which
      # cannot decompress into a preallocated buffer. Applies to the whole
      # tree, which is why the figure above is uncompressed.
      hardware.firmwareCompression = "none";
    }

    (lib.mkIf cfg.firmware.enable {
      hardware.firmware = [pkgs.firmware-surface-pro-11];
    })
  ]);
}
