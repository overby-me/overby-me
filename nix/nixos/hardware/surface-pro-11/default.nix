# Microsoft Surface Pro 11 (Qualcomm Snapdragon X Elite, board name "Denali").
#
# Vendored from https://github.com/andre4ik3/nixos-surface-pro-11, which is in
# turn based on the work of @dwhinham and @jhovold.  That flake is unmaintained
# (last push 2025-09-27) and pins an out-of-tree kernel at 6.17.0-rc3, so only
# the parts with no upstream replacement are carried over.
#
# What was dropped and why: the custom kernel.  Dale Whinham upstreamed the
# Surface Pro 11 devicetrees in Linux 7.0, and mainline's
# surface_aggregator_registry matches "microsoft,denali", so nixpkgs'
# linuxPackages_latest boots this machine with every needed driver already
# enabled.  See hardware.nix.
#
# Known gaps that follow from that.  dwhinham's branch is roughly three
# relevant commits ahead of mainline, and none of them is worth a full aarch64
# kernel rebuild pre-emptively; each can be revisited with boot.kernelPatches.
#
#   Wi-Fi may come up hard-blocked.  That tree adds an ath12k `disable-rfkill`
#   devicetree property; mainline only has the ACPI equivalent
#   (ath12k_acpi_get_disable_rfkill), which a devicetree-booted machine never
#   reaches.  Try `rfkill unblock all` first.
#
#   Suspend and resume.  There is a commit there titled "HACK: Workaround
#   suspend/resume issues on SP11".  Expect trouble.
#
#   eDP link rates.  "drm/msm/dp: Enable support for eDP v1.4+ link rates
#   table" is not upstream, which may cap the OLED panel's refresh modes.
#
# The fourth out-of-tree commit, setting the Wi-Fi MAC address from the
# devicetree, is not a gap: networking.nix does that from userspace instead.
{...}: {
  imports = [
    ./hardware.nix
    ./firmware.nix
    ./networking.nix
  ];

  # Inject the firmware package into pkgs via an overlay so the sub-modules can
  # reference it as a plain `pkgs.<name>`.
  nixpkgs.overlays = [
    (final: _prev: {
      firmware-surface-pro-11 = final.callPackage ./pkgs/firmware-surface-pro-11.nix {};
    })
  ];
}
