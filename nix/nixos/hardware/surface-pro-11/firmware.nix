# Surface Pro 11 firmware.
#
# Off by default: the blobs are unfree and are fetched from a mirror of
# Microsoft's Windows driver packages, so opting in is a deliberate act.  The
# GPU, both DSPs, Wi-Fi and Bluetooth all stay dark without them.
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

  config = lib.mkIf (cfg.enable && cfg.firmware.enable) {
    hardware = {
      firmware = [pkgs.firmware-surface-pro-11];
      # Qualcomm remoteproc images are loaded by the firmware loader without
      # decompression support, so they must be installed uncompressed.
      firmwareCompression = "none";
    };
  };
}
