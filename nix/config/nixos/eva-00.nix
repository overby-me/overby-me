# ╔═══════════════════════════════════════════════════════════════════════╗
# ║  EVA-00 — PROTOTYPE                                                 ║
# ║                                                                      ║
# ║  "Man fears the darkness, and so he scrapes away at the edges of     ║
# ║   it with fire."  — Rei Ayanami                                     ║
# ║                                                                      ║
# ║  Raspberry Pi 5 · Headless · IronClaw AI Agent                      ║
# ╚═══════════════════════════════════════════════════════════════════════╝
#
# Deployment:
#   1. Build the installer image from nixos-raspberrypi and flash to SD/NVMe
#      nix build github:nvmd/nixos-raspberrypi#installerImages.rpi5
#   2. Boot the RPi5, find it via mDNS (eva-00.local) or DHCP lease
#   3. Deploy this configuration:
#      nixos-rebuild switch --flake .#eva-00 --target-host root@eva-00.local
#   4. On first run, SSH in and complete IronClaw onboarding:
#      sudo -u ironclaw ironclaw onboard
#   5. Populate the environment file with secrets:
#      See services.ironclaw.environmentFile below
#
# Note: This configuration is built via nixos-raspberrypi.lib.nixosSystem
# (not nixpkgs.lib.nixosSystem) because nixos-raspberrypi requires a
# patched nixpkgs where mkRemovedOptionModule generates module keys so the
# deprecated boot.loader.raspberryPi option from nixpkgs' rename.nix can
# be selectively disabled and replaced.  Flakelight's nixosConfigurations
# handler detects already-built systems via the `isNixos` check
# (x ? config.system.build.toplevel) and passes them through without
# re-wrapping in nixpkgs.lib.nixosSystem.
#
# IMPORTANT: Do NOT pass `lib` from flakelight's module args into
# specialArgs — that would inject the unpatched nixpkgs lib and break the
# patched disabledModules mechanism that resolves the boot.loader conflict.
# The self overlay is added explicitly so pkgs.ironclaw is available.
#
# Hardware note:
#   The fileSystems block below assumes the partition layout created by
#   the nixos-raspberrypi installer image.  If you partition manually or
#   use NVMe, update the device paths / UUIDs accordingly.
{
  inputs,
  src,
  ...
}:
inputs.nixos-raspberrypi.lib.nixosSystem {
  specialArgs = {
    inherit (inputs) nixos-raspberrypi;
    inherit src inputs;
    stateVersion = "25.05";
  };

  modules = [
    # ── Board support (from nixos-raspberrypi) ──────────────────────────
    inputs.nixos-raspberrypi.nixosModules.raspberry-pi-5.base
    inputs.nixos-raspberrypi.nixosModules.raspberry-pi-5.page-size-16k
    inputs.nixos-raspberrypi.nixosModules.raspberry-pi-5.bluetooth

    # ── Secrets ─────────────────────────────────────────────────────────
    inputs.ragenix.nixosModules.default

    # ── Overlays (make pkgs.ironclaw etc. available) ────────────────────
    {nixpkgs.overlays = [inputs.self.overlays.default];}

    # ── zswap ──────────────────────────────────────────────────────────
    ../../modules/nixos/core/zswap.nix

    # ── IronClaw service module ─────────────────────────────────────────
    ../../modules/nixos/services/ironclaw.nix

    # ── Machine configuration ───────────────────────────────────────────
    ({
      config,
      pkgs,
      lib,
      inputs,
      stateVersion,
      ...
    }: let
      inherit (inputs.self.secrets) publicKeys;
    in {
      # ── Identity ────────────────────────────────────────────────────
      networking.hostName = "eva-00";
      system = {
        inherit stateVersion;

        # Tag generations for easy identification
        nixos.tags = let
          cfg = config.boot.loader.raspberry-pi;
        in [
          "evangelion-prototype"
          "raspberry-pi-${cfg.variant}"
          cfg.bootloader
          config.boot.kernelPackages.kernel.version
        ];
      };

      # ── Boot ────────────────────────────────────────────────────────
      # Use the new generational bootloader (recommended for new RPi5
      # installations, see nixos-raspberrypi#60)
      boot.loader.raspberry-pi.bootloader = "kernel";

      # ── Filesystem ──────────────────────────────────────────────────
      # Matches the default installer-image partition layout.
      # TODO: Replace with actual UUIDs from `blkid` after first boot.
      fileSystems = {
        "/" = {
          device = "/dev/disk/by-label/NIXOS_SD";
          fsType = "ext4";
          options = ["noatime"];
        };

        "/boot/firmware" = {
          device = "/dev/disk/by-label/FIRMWARE";
          fsType = "vfat";
          options = ["fmask=0077" "dmask=0077"];
        };
      };

      # ── Firmware & swap ─────────────────────────────────────────────
      hardware.enableRedistributableFirmware = true;
      zswap.enable = true;

      # ── Locale & time ───────────────────────────────────────────────
      time.timeZone = "Europe/Copenhagen";
      i18n.defaultLocale = "en_DK.UTF-8";

      # ── Networking ──────────────────────────────────────────────────
      networking = {
        useDHCP = lib.mkDefault true;
        networkmanager.enable = false;
        # Use systemd-networkd for a headless server
        useNetworkd = true;
        firewall = {
          enable = true;
          allowedTCPPorts = [
            22 # SSH
            3000 # IronClaw web gateway
          ];
        };
      };

      # ── Services ────────────────────────────────────────────────────
      services = {
        # mDNS so the device is discoverable as eva-00.local
        avahi = {
          enable = true;
          nssmdns4 = true;
          publish = {
            enable = true;
            addresses = true;
          };
        };

        resolved = {
          enable = true;
          extraConfig = ''
            MulticastDNS=resolve
          '';
        };

        # SSH
        openssh = {
          enable = true;
          settings = {
            PermitRootLogin = "prohibit-password";
            PasswordAuthentication = false;
          };
        };

        # IronClaw AI agent
        #
        # "The interaction of men and women isn't very logical."
        #   — Rei Ayanami
        #
        # IronClaw is a secure, self-expanding AI assistant.  On EVA-00 it
        # runs as a persistent daemon with its own PostgreSQL database,
        # WASM-sandboxed tool execution, and a web gateway on port 3000.
        #
        # Secrets are supplied via an environment file so they never touch
        # the Nix store.  Create /var/lib/ironclaw/env with at minimum:
        #
        #   LLM_BACKEND=nearai
        #   NEARAI_API_KEY=<your key>
        #
        # Or use agenix to manage the file declaratively:
        #
        #   age.secrets."ironclaw-env" = {
        #     file = inputs.self.secrets.ironclaw-env;
        #     path = "/run/agenix/ironclaw-env";
        #     owner = "ironclaw";
        #   };
        #   services.ironclaw.environmentFile = config.age.secrets."ironclaw-env".path;
        #
        ironclaw = {
          enable = true;
          logLevel = "ironclaw=info";
          environmentFile = "/var/lib/ironclaw/env";
        };

        # Journal to volatile storage to reduce SD card writes
        journald.extraConfig = ''
          Storage=volatile
          RuntimeMaxUse=64M
        '';
      };

      # ── Users ───────────────────────────────────────────────────────
      users.users = {
        root.openssh.authorizedKeys.keys = [publicKeys.noverby-ssh-ed25519];

        noverby = {
          isNormalUser = true;
          description = "Niclas Overby";
          extraGroups = ["wheel"];
          openssh.authorizedKeys.keys = [publicKeys.noverby-ssh-ed25519];
        };
      };

      security.sudo.wheelNeedsPassword = false;

      # ── Nix settings ────────────────────────────────────────────────
      nix = {
        settings = {
          trusted-users = ["root" "noverby"];
          experimental-features = "nix-command flakes";
          substituters = [
            "https://overby-me.cachix.org"
            "https://nixos-raspberrypi.cachix.org"
          ];
          trusted-public-keys = [
            "overby-me.cachix.org-1:dU7qOj5u97QZz98nqnh+Nwait6c+2d2Eq0KTOAXTyp4="
            "nixos-raspberrypi.cachix.org-1:4iMO9LXa8BqhU+Rpg6LQKiGa2lsNh/j2oiYLNOQ5sPI="
          ];
        };

        # Collect garbage weekly to keep the SD card from filling up
        gc = {
          automatic = true;
          dates = "weekly";
          options = "--delete-older-than 14d";
        };
      };

      # ── Monitoring / quality of life ────────────────────────────────
      environment.systemPackages = with pkgs; [
        helix # editor
        htop # process monitor
        bottom # system monitor
        tree # directory listing
        jq # JSON wrangling
        ripgrep # search
        usbutils # lsusb
        pciutils # lspci
        raspberrypi-utils # vcgencmd, etc.
      ];
    })
  ];
}
