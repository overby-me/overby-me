# Surface Pro 11 firmware, from two sources.
#
# linux-firmware, always: the Adreno X1-85 needs gen70500_sqe.fw,
# gen70500_gmu.bin and x1e80100/gen70500_zap.mbn, and without them nothing
# about the display works.  The zap shader is the one that bites, because it
# is what moves the GPU out of its secure boot state.  Miss it and programming
# the GPU's own SMMU times out:
#
#   arm-smmu 3da0000.iommu: probe with driver arm-smmu failed with error -110
#   adreno 3d00000.gpu: deferred probe timeout, ignoring dependency
#   msm_dpu ae01000.display-controller: failed to load adreno gpu
#   msm_dpu ae01000.display-controller: failed to bind 3d00000.gpu: -19
#
# msm_dpu then never binds, simpledrm keeps /dev/dri/card0 with no render
# node, and the first compositor to start spins on it at 100%+ CPU with a
# black screen.  These blobs are redistributable, so they are not gated
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
      # Adreno microcode and zap shader.  1.7 GiB of linux-firmware for three
      # files is poor value, but the alternative is copying them out by hand
      # and rediscovering the next missing one the same slow way.
      hardware.enableRedistributableFirmware = true;

      # Qualcomm remoteproc images are loaded with request_firmware_into_buf,
      # which cannot decompress into a preallocated buffer, so they have to be
      # installed uncompressed.  This applies to the whole firmware tree, which
      # is why the figure above is uncompressed.
      hardware.firmwareCompression = "none";
    }

    (lib.mkIf cfg.firmware.enable {
      hardware.firmware = [pkgs.firmware-surface-pro-11];
    })
  ]);
}
