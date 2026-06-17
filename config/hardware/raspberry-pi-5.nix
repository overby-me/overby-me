# Raspberry Pi 5 hardware module
#
# Imports board support from the nixos-raspberrypi flake and provides
# a baseline hardware configuration suitable for a headless RPi5.
# Machine-specific settings (filesystems, hostName, …) belong in the
# corresponding nixosConfiguration under config/nixos/.
{
  lib,
  inputs,
  ...
}: {
  imports = [
    # nixos-raspberrypi board support
    inputs.nixos-raspberrypi.lib.inject-overlays
    inputs.nixos-raspberrypi.nixosModules.trusted-nix-caches
    inputs.nixos-raspberrypi.nixosModules.nixpkgs-rpi
    inputs.nixos-raspberrypi.nixosModules.raspberry-pi-5.base
    inputs.nixos-raspberrypi.nixosModules.raspberry-pi-5.page-size-16k
    inputs.nixos-raspberrypi.nixosModules.raspberry-pi-5.bluetooth
  ];

  # Enable wireless firmware shipped by nixos-raspberrypi
  hardware.enableRedistributableFirmware = true;

  # zswap – compressed swap cache backed by real swap
  zswap = {
    enable = true;
    swapSize = 8 * 1024;
  };

  # DHCP by default; override per-interface in the machine config if needed
  networking.useDHCP = lib.mkDefault true;

  nixpkgs.hostPlatform = lib.mkDefault "aarch64-linux";
}
