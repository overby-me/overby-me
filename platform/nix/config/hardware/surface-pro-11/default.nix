# Microsoft Surface Pro 11 (Snapdragon X Elite, board "Denali").
#
# Vendored from https://github.com/andre4ik3/nixos-surface-pro-11 (itself based
# on @dwhinham and @jhovold), unmaintained since 2025-09-27 and pinning an
# out-of-tree 6.17.0-rc3. Only the parts with no upstream replacement are kept:
# the devicetrees landed in Linux 7.0 and mainline's surface_aggregator_registry
# matches "microsoft,denali", so linuxPackages_latest boots this machine.
#
# Three commits on that branch are still ahead of mainline, none worth an
# aarch64 kernel rebuild pre-emptively. Each is reachable via boot.kernelPatches:
#
#   Wi-Fi may come up hard-blocked. That tree adds an ath12k `disable-rfkill`
#   devicetree property; mainline has only the ACPI equivalent, which a
#   devicetree-booted machine never reaches. Try `rfkill unblock all` first.
#
#   Suspend and resume: "HACK: Workaround suspend/resume issues on SP11".
#
#   eDP link rates: "drm/msm/dp: Enable support for eDP v1.4+ link rates table"
#   is not upstream, which may cap the OLED panel's refresh modes.
#
# Its fourth out-of-tree commit sets the Wi-Fi MAC from the devicetree; that one
# is not a gap, because networking.nix does it from userspace.
{...}: {
  imports = [
    ./hardware.nix
    ./firmware.nix
    ./networking.nix
    ./audio.nix
  ];

  # Through an overlay so the sub-modules can take them as plain `pkgs.<name>`.
  nixpkgs.overlays = [
    (final: _prev: {
      firmware-surface-pro-11 = final.callPackage ./pkgs/firmware-surface-pro-11.nix {};
      audio-topology-surface-pro-11 = final.callPackage ./pkgs/audio-topology-surface-pro-11.nix {};
      alsa-ucm-conf-surface-pro-11 = final.callPackage ./pkgs/alsa-ucm-conf-surface-pro-11.nix {};
    })
  ];
}
