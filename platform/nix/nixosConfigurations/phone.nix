# ╔═══════════════════════════════════════════════════════════════════════╗
# ║  PHONE — Fairphone 5                                                ║
# ║                                                                      ║
# ║  NixOS on Fairphone 5 (QCM6490 / Qualcomm SC7280) with COSMIC DE.  ║
# ║  Built natively for aarch64-linux via binfmt emulation on x86_64.   ║
# ║                                                                      ║
# ║  Based on: https://github.com/gian-reto/nixos-fairphone-fp5         ║
# ╚═══════════════════════════════════════════════════════════════════════╝
#
# Flashing (first time):
#   1. Unlock the bootloader (see PostmarketOS wiki for Fairphone 5)
#   2. Put device into fastboot mode (power off, hold volume-down + power)
#   3. Connect via USB-C
#   4. Build & flash boot image:
#        nix build .#nixosConfigurations.phone.config.system.build.kernel
#   5. Build & flash rootfs image (ext4 for userdata partition)
#
# Subsequent updates (over SSH / on-device):
#   nixos-rebuild switch --flake .#phone --target-host root@<phone-ip>
#
# Note on build strategy:
#   Rather than cross-compiling (nixpkgs.buildPlatform = "x86_64-linux"),
#   this configuration builds natively for aarch64-linux.  The x86_64 host
#   must have binfmt emulation enabled (boot.binfmt.emulatedSystems =
#   ["aarch64-linux"]).  This way:
#     - Pre-built aarch64-linux packages are fetched from the binary cache
#     - Only uncached packages (kernel, firmware, etc.) are built under
#       QEMU emulation — slower per-package but far fewer to build
#     - No cross-compilation workaround overlays are needed
{
  inputs,
  src,
  lib,
  ...
}: {
  system = "aarch64-linux";

  specialArgs = {
    inherit src inputs lib;
    stateVersion = "25.11";
    hasSecrets = true;
  };

  modules = [
    # ── Fairphone 5 hardware support ──────────────────────────────────
    inputs.self.hardware.fairphone-fp5
    {
      nixos-fairphone-fp5.serial = {
        enable = false;
        verbose = true;
      };
    }

    # ── Home Manager ──────────────────────────────────────────────────
    inputs.home-manager.nixosModules.home-manager
    inputs.self.nixosModules.home-manager

    # ── Secrets ───────────────────────────────────────────────────────
    inputs.ragenix.nixosModules.default
    ({pkgs, ...}: {
      # Simplified age config for the phone (no FIDO2/Nitrokey needed).
      # Decryption uses the pre-generated SSH host key injected into the
      # rootfs image at build time.  The key is stored age-encrypted in
      # secrets/phone-host-key.age and must be decrypted before
      # building the rootfs image:
      #   rage -d -i ~/.ssh/id_ed25519 secrets/phone-host-key.age \
      #     -o /tmp/phone-hostkeys/ssh_host_ed25519_key
      # The mkRootfsImage populateImageCommands then copies it into the
      # ext4 image at /etc/ssh/.
      age = {
        ageBin = "${pkgs.rage}/bin/rage";
        identityPaths = ["/etc/ssh/ssh_host_ed25519_key"];
      };
    })

    # ── Desktop environment ───────────────────────────────────────────
    inputs.self.desktops.cosmic

    # ── Machine configuration ─────────────────────────────────────────
    ({
      pkgs,
      inputs,
      ...
    }: let
      inherit (inputs.self.secrets) publicKeys;
    in {
      nixpkgs.overlays = [
        inputs.self.overlays.default
      ];

      # ── Identity ────────────────────────────────────────────────────
      networking.hostName = "phone";

      # ── Nix settings ────────────────────────────────────────────────
      nix.settings = {
        experimental-features = ["nix-command" "flakes"];
        # As in platform/nix/nixosModules/core/nix.nix, which this host does not import:
        # a .drv is a GC root for the whole build-time closure of what it built,
        # and a phone has the least room to spare for one.
        keep-derivations = false;
        # When on the device, prefer remote builders to avoid draining
        # battery and running out of memory during local rebuilds.
        trusted-users = ["root" "overby.me"];
      };

      # Disable NixOS manual (saves closure size and hides desktop icon).
      documentation.nixos.enable = false;

      # ── Networking ──────────────────────────────────────────────────
      networking = {
        wireless.iwd.enable = true;
        networkmanager = {
          enable = true;
          wifi.backend = "iwd";
        };
        firewall.enable = true;
      };

      # Auto-login: COSMIC greeter has no on-screen keyboard, so we
      # bypass it to land directly on the desktop via touchscreen.
      services.displayManager.autoLogin = {
        enable = true;
        user = "overby.me";
      };

      # Pre-configured WiFi network for headless SSH access.
      age.secrets."wifi-concero.nmconnection" = {
        file = inputs.self.secrets.wifi-concero;
        path = "/etc/NetworkManager/system-connections/Concero.nmconnection";
        owner = "root";
        group = "root";
        mode = "0600";
      };

      # ── Bluetooth ───────────────────────────────────────────────────
      hardware.bluetooth = {
        enable = true;
        powerOnBoot = true;
      };

      # Blueman crashes under cross-compilation (missing GTK 3 typelibs)
      # and COSMIC's bluetooth applet does not register a BlueZ agent,
      # so pairing always fails with AuthenticationCanceled.  Run
      # bt-agent from bluez-tools as a lightweight daemon that registers
      # a BlueZ agent with KeyboardDisplay capability.  A wrapper script
      # monitors bt-agent's stdout for passkey lines and sends desktop
      # notifications via notify-send so the user can see the passkey
      # on screen and type it on the BT keyboard.
      systemd.user.services.bt-agent = let
        bt-agent-notify = pkgs.writeShellScript "bt-agent-notify" ''
          ${pkgs.bluez-tools}/bin/bt-agent -c KeyboardDisplay 2>&1 | while IFS= read -r line; do
            printf '%s\n' "$line"
            case "$line" in
              *Passkey:*|*passkey:*|*PIN:*|*pin:*)
                ${pkgs.libnotify}/bin/notify-send -u critical -t 30000 \
                  "Bluetooth Pairing" "$line" || true
                ;;
            esac
          done
        '';
      in {
        description = "Bluetooth pairing agent";
        after = ["bluetooth.target"];
        wantedBy = ["default.target"];
        serviceConfig = {
          Type = "simple";
          Restart = "on-failure";
          RestartSec = 5;
          ExecStart = "${bt-agent-notify}";
        };
      };

      # ── Audio (PipeWire) ────────────────────────────────────────────
      security.rtkit.enable = true;

      # ── Display / GPU ───────────────────────────────────────────────
      hardware.graphics.enable = true;

      # ── Services ────────────────────────────────────────────────────
      services = {
        pipewire = {
          enable = true;
          alsa.enable = true;
          pulse.enable = true;
        };

        openssh = {
          enable = true;
          settings = {
            PermitRootLogin = "yes";
            PasswordAuthentication = true;
          };
        };

        upower.enable = true;
        thermald.enable = false; # Not applicable on ARM

        # ── USB-C host mode (OTG) ──────────────────────────────────────
        # The Qualcomm PMIC firmware defaults the USB-C port to device
        # (UFP) mode.  The UCSI layer reports the state but does not
        # automatically initiate a data-role swap to host (DFP) when a
        # USB peripheral (keyboard, hub, …) is connected via USB-C.
        #
        # This udev rule detects Type-C partner connect/disconnect events
        # and switches data_role to "host" so the DWC3 controller brings
        # up xHCI and enumerates the attached USB device.  If the partner
        # doesn't support DRP the write fails harmlessly.
        udev.extraRules = ''
          ACTION=="change", SUBSYSTEM=="typec", KERNEL=="port[0-9]*", \
            ATTR{data_role}=="host [device]", \
            ATTR{data_role}="host"
        '';
      };

      # ── Useful packages ─────────────────────────────────────────────
      environment.systemPackages = with pkgs; [
        # System utilities
        htop

        # Networking (blueman removed — crashes under cross-compilation)
        # notify-send for desktop notifications (used by bt-agent wrapper)
        libnotify

        # On-screen keyboard (COSMIC doesn't have one built-in yet)
        cosmic-osk
      ];

      # ── User ────────────────────────────────────────────────────────
      users = {
        mutableUsers = true;

        users."overby.me" = {
          isNormalUser = true;
          description = "Niclas Overby";
          initialPassword = "changeme";
          extraGroups = [
            "networkmanager"
            "video"
            "wheel"
          ];
          openssh.authorizedKeys.keys = [publicKeys.overby-me-ssh-ed25519];
        };
      };

      security.sudo.wheelNeedsPassword = false;

      # ── System ──────────────────────────────────────────────────────
      system.stateVersion = "25.11";
    })
  ];
}
