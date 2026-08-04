# ╔═══════════════════════════════════════════════════════════════════════╗
# ║  ARMITAS · Surface Pro 11                                            ║
# ║                                                                       ║
# ║  NixOS on the Microsoft Surface Pro 11 (Qualcomm Snapdragon X Elite,  ║
# ║  board name "Denali") with COSMIC.  Built natively for aarch64-linux  ║
# ║  via binfmt emulation on x86_64.                                      ║
# ║                                                                       ║
# ║  Hardware support vendored from:                                      ║
# ║  https://github.com/andre4ik3/nixos-surface-pro-11                    ║
# ╚═══════════════════════════════════════════════════════════════════════╝
#
# Installation.  The disk is wiped, so everything below happens once and the
# only thing typed on the tablet is a Wi-Fi password.
#
#   1. In Windows, while it is still there: disable BitLocker, and note the
#      MAC addresses from `ipconfig /all` (see
#      nixos/hardware/surface-pro-11/networking.nix).  They are the one thing
#      that cannot be recovered afterwards.
#   2. Disable Secure Boot in UEFI (hold volume-up while powering on).
#   3. Build and write the installer:
#        just -f nix/nixos/justfile build-iso
#        sudo dd if=result-armitas-iso/iso/*.iso of=/dev/sdX bs=4M status=progress
#   4. Boot it.  Putting USB at the top of the boot order is not enough: hold
#      volume-up and swipe left on the USB entry each time.  The ISO carries
#      the devicetree in its boot entry, so there is nothing to type at GRUB.
#   5. On the tablet, join a network with `nmtui` and read off its address
#      with `ip -brief addr`.
#   6. From this workstation, one command does the rest:
#        just -f nix/nixos/justfile install-armitas <tablet-ip>
#      It partitions and formats per the disko layout below, copies the
#      closure built here, installs, and reboots.
#
# Subsequent updates:
#   nixos-rebuild switch --flake .#armitas --target-host root@armitas
#
# Note on build strategy:
#   As with `phone`, this builds natively for aarch64-linux rather than
#   cross-compiling, so the x86_64 host needs
#   `boot.binfmt.emulatedSystems = ["aarch64-linux"]` (gravitas has it via
#   nixos/modules/core/boot.nix).  Nearly everything, the 7.1.2 kernel
#   included, then substitutes from cache.nixos.org instead of being built.
{
  inputs,
  src,
  lib,
  ...
}: {
  system = "aarch64-linux";

  specialArgs = {
    inherit src inputs lib;
    stateVersion = "26.05";
    # No armitas host key exists yet, so nothing in nixos/secrets is
    # encrypted to this machine and boot-time decryption would fail.
    # To turn this on: install, then add the generated
    # /etc/ssh/ssh_host_ed25519_key.pub to nixos/secrets/publicKeys.nix as
    # `armitas-ssh-ed25519`, include it in `all`, run `just -f nix/nixos/justfile
    # rekey`, and flip this to true.
    hasSecrets = false;
  };

  modules = [
    # ── Surface Pro 11 hardware support ───────────────────────────────
    inputs.self.hardware.surface-pro-11

    # ── Disk layout ───────────────────────────────────────────────────
    # Also generates fileSystems, so there is nothing to hand-write and
    # nothing to keep in sync with what was actually formatted.
    inputs.disko.nixosModules.disko

    # ── Home Manager ──────────────────────────────────────────────────
    inputs.home-manager.nixosModules.home-manager
    inputs.self.nixosModules.home-manager

    # ── Secrets ───────────────────────────────────────────────────────
    inputs.ragenix.nixosModules.default
    inputs.self.nixosModules.age

    # ── Desktop environment ───────────────────────────────────────────
    inputs.self.desktops.cosmic

    # ── Shared configuration ──────────────────────────────────────────
    # The portable slice of nixos/modules/core.  Deliberately not the whole
    # `core` module: core/boot.nix pins the x86-only zen kernel and enables
    # aarch64 binfmt, core/hardware.nix wants amdgpu and a Nitrokey, and
    # core/virtualisation.nix pulls in docker, libvirtd and waydroid.  The
    # bootloader settings core/boot.nix carried are restated below.
    ../modules/core/audio.nix
    ../modules/core/console.nix
    ../modules/core/environment.nix
    ../modules/core/fonts.nix
    ../modules/core/locale.nix
    ../modules/core/networking.nix
    ../modules/core/nix.nix
    ../modules/core/secrets.nix
    ../modules/core/system.nix
    ../modules/core/users.nix
    ../modules/core/zram.nix
    ../modules/services/openssh.nix
    ../modules/services/resolved.nix

    # ── Machine configuration ─────────────────────────────────────────
    (_: {
      # ── Identity ────────────────────────────────────────────────────
      networking.hostName = "armitas";

      # ── Boot ────────────────────────────────────────────────────────
      # The Surface Pro 11's UEFI describes the machine with ACPI, for
      # Windows, so Linux needs the devicetree from the bootloader.
      # systemd-boot writes the `devicetree` line into the loader entry by
      # itself once hardware.deviceTree.name is set, which the hardware
      # module does.
      boot = {
        loader = {
          timeout = 3;
          systemd-boot = {
            enable = true;
            configurationLimit = 10;
            editor = false;
          };
          efi.canTouchEfiVariables = true;
        };
        consoleLogLevel = 0;
        initrd.verbose = false;
        kernelParams = [
          "boot.shell_on_fail"
          "loglevel=3"
          "quiet"
          "rd.systemd.show_status=false"
          "rd.udev.log_level=3"
          "udev.log_priority=3"
        ];
      };

      # ── Disk ────────────────────────────────────────────────────────
      # Whole-disk GPT, no Windows, no encryption.  Encryption is left off
      # deliberately: the GPU firmware times out a few seconds into boot, so
      # a passphrase prompt that is not answered promptly lands you on a
      # black screen.  disko derives fileSystems from this.
      #
      # DESTRUCTIVE.  `disko --mode destroy,format,mount` and nixos-anywhere
      # both wipe /dev/nvme0n1 without asking.
      disko.devices.disk.main = {
        device = "/dev/nvme0n1";
        type = "disk";
        content = {
          type = "gpt";
          partitions = {
            ESP = {
              priority = 1;
              type = "EF00";
              # Roomy on purpose: systemd-boot copies a kernel and an initrd
              # per generation to the ESP, and configurationLimit is 10.
              size = "2G";
              content = {
                type = "filesystem";
                format = "vfat";
                mountpoint = "/boot";
                mountOptions = ["umask=0077"];
              };
            };
            root = {
              size = "100%";
              content = {
                type = "filesystem";
                format = "ext4";
                mountpoint = "/";
                mountOptions = ["noatime"];
              };
            };
          };
        };
      };

      # ── Hardware ────────────────────────────────────────────────────
      hardware = {
        graphics.enable = true;

        bluetooth = {
          enable = true;
          powerOnBoot = true;
        };

        # The blobs behind the GPU, both DSPs, Wi-Fi and Bluetooth.  Fill in
        # the MAC addresses from Windows to keep DHCP reservations and
        # Bluetooth pairings stable across the two operating systems.
        surfacePro11 = {
          firmware.enable = true;
          wireless.enable = true;
          # wireless.macAddress = "XX:XX:XX:XX:XX:XX";
          bluetooth.enable = true;
          # bluetooth.macAddress = "XX:XX:XX:XX:XX:XX";
        };
      };

      # ── Networking ──────────────────────────────────────────────────
      # COSMIC's applet drives NetworkManager; iwd is the backend the
      # hardware module configures for this Wi-Fi chip.
      networking = {
        networkmanager.wifi.backend = "iwd";
        firewall.enable = true;
      };

      # ── Swap ────────────────────────────────────────────────────────
      zram.enable = true;

      # ── Power ───────────────────────────────────────────────────────
      services.upower.enable = true;
    })
  ];
}
