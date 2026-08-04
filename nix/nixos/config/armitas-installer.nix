# ╔═══════════════════════════════════════════════════════════════════════╗
# ║  ARMITAS INSTALLER · Surface Pro 11 boot media                       ║
# ║                                                                       ║
# ║  Minimal NixOS installer ISO carrying the Surface Pro 11 hardware     ║
# ║  module, Linux 7.1.2 and the Denali firmware, so the installer has    ║
# ║  a working display, keyboard and Wi-Fi.                               ║
# ╚═══════════════════════════════════════════════════════════════════════╝
#
# Build:
#   just -f nix/nixos/justfile build-iso
#
# The Surface Pro 11's UEFI hands Linux no devicetree, and stock nixpkgs
# cannot put a `devicetree` line in the ISO's GRUB entry, so booting it
# normally means editing the entry by hand at the menu.  Importing
# nixosModules.iso-image swaps in a copy of that module carrying
# NixOS/nixpkgs#396334, which emits the line automatically.  Nothing to type.
#
# The full dtbs tree is on the media as well.  Nothing needs it to boot, but
# if this machine ever turns out to be the LCD/X Plus model, its devicetree
# is one `e` away at the menu: x1p64100-microsoft-denali.dtb.
#
# Note: this is built with nixpkgs.lib.nixosSystem rather than handed to
# flakelight as a { system, modules } attrset, and that is load-bearing.
# nix/flake/modules/colmena.nix registers every nixosConfiguration that is
# *not* already a built system as a deploy target, so an installer image
# would otherwise show up as a node at armitas-installer.overby.me.
# Flakelight's own handler detects built systems via
# `x ? config.system.build.toplevel` and passes them through untouched, the
# same way eva-00 is handled.
{inputs, ...}:
inputs.nixpkgs.lib.nixosSystem {
  system = "aarch64-linux";

  specialArgs = {inherit inputs;};

  modules = [
    ({
      config,
      lib,
      modulesPath,
      ...
    }: let
      inherit (inputs.self.secrets) publicKeys;
    in {
      imports = [
        # The minimal installer plus linuxPackages_latest.  The `latest` part
        # matters: the Surface Pro 11 devicetrees only exist from Linux 7.0,
        # and 26.05's default kernel is 6.18.  The -no-zfs variant, because
        # the installer profiles turn ZFS on and openzfs 2.4.2 does not build
        # against 7.1.
        (modulesPath + "/installer/cd-dvd/installation-cd-minimal-new-kernel-no-zfs.nix")
        inputs.self.hardware.surface-pro-11

        # Replaces nixpkgs' iso-image.nix with a devicetree-capable copy.
        # Imported after the installer profile above, which pulls in the
        # stock module that this one disables.
        inputs.self.nixosModules.iso-image
      ];

      # Built outside flakelight's mkNixos, so flakelight's propagationModule
      # does not forward the flake's nixpkgs.config here.  The Denali firmware
      # is unfree and would refuse to evaluate without this.
      nixpkgs.config.allowUnfree = true;

      # Wi-Fi during the install.  Leaving the MAC addresses unset is fine
      # here; they only matter for the installed system.
      hardware.surfacePro11 = {
        firmware.enable = true;
        wireless.enable = true;
      };

      # profiles/installation-device.nix enables NetworkManager, which in turn
      # enables wpa_supplicant unless told otherwise, and NixOS refuses to run
      # two wireless daemons alongside the iwd the hardware module sets up.
      # Point NetworkManager at iwd instead of forcing wpa_supplicant off, so
      # `nmtui` still works in the installer.
      networking.networkmanager.wifi.backend = "iwd";

      # The patched module already places the one devicetree the boot entry
      # names.  This adds the rest of the tree, purely as an escape hatch for
      # the wrong-model case described in the header.
      isoImage.contents = [
        {
          source = "${config.hardware.deviceTree.kernelPackage}/dtbs";
          target = "/boot/dtbs/${config.hardware.deviceTree.kernelPackage.modDirVersion}";
        }
      ];

      # nixos-minimal-26.05.<rev>-armitas, rather than the default which ends
      # in the bare system name.
      image.baseName = lib.mkForce "nixos${
        lib.optionalString (config.isoImage.edition != "") "-${config.isoImage.edition}"
      }-${config.system.nixos.label}-armitas";

      # installation-cd-base.nix turns this on unconditionally, but
      # memtest86plus is x86-only, so on aarch64 it only adds a menu entry
      # pointing at a file the ISO does not contain.
      boot.loader.grub.memtest86.enable = lib.mkForce false;

      # Headless installs over the network.
      services.openssh.enable = true;
      users.users.root.openssh.authorizedKeys.keys = [publicKeys.overby-me-ssh-ed25519];
    })
  ];
}
