# Surface Pro 11 Wi-Fi and Bluetooth.
#
# Both radios sit behind the WCN7850 combo chip and need the firmware from
# firmware.nix.  Neither has its MAC address in an EEPROM the driver can read,
# so each boot they come up on whatever the firmware defaults to.  Windows
# stores the real addresses; set them here to keep DHCP reservations, captive
# portals and Bluetooth pairings stable across the two operating systems.
#
# Read the addresses in Windows with `ipconfig /all`:
#   Wireless LAN adapter Wi-Fi                     -> wireless.macAddress
#   Ethernet adapter Bluetooth Network Connection  -> bluetooth.macAddress
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.hardware.surfacePro11;
in {
  options.hardware.surfacePro11 = {
    wireless = {
      enable = lib.mkEnableOption "Wi-Fi support on the Surface Pro 11";
      macAddress = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "AA:BB:CC:DD:EE:FF";
        description = ''
          The MAC address to use when configuring Wi-Fi.  Should match the MAC
          address recorded in Windows, but can be any valid MAC address.  Left
          null the interface keeps whatever address the firmware hands it.
        '';
      };
    };
    bluetooth = {
      enable = lib.mkEnableOption "Bluetooth support on the Surface Pro 11";
      macAddress = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "AA:BB:CC:DD:EE:FF";
        description = ''
          The MAC address to use when configuring Bluetooth.  Should match the
          MAC address recorded in Windows, but can be any valid MAC address.
          Left null the controller keeps whatever address the firmware hands
          it.
        '';
      };
    };
  };

  config = lib.mkIf cfg.enable (lib.mkMerge [
    (lib.mkIf (cfg.wireless.enable || cfg.bluetooth.enable) {
      assertions = [
        {
          assertion = cfg.firmware.enable;
          message = ''
            Wi-Fi and Bluetooth support on the Surface Pro 11 require firmware.
          '';
        }
      ];
    })

    (lib.mkIf cfg.wireless.enable {
      networking.wireless.iwd = {
        enable = true;
        settings = {
          # Bug workaround: https://bugzilla.kernel.org/show_bug.cgi?id=218733
          General.ControlPortOverNL80211 = false;
        };
      };
    })

    # Matched on the driver rather than on a PCI address, and applied to $name
    # rather than to a fixed wlP6p1s0: neither the PCI domain nor the
    # predictable interface name is guaranteed to survive a kernel bump.
    (lib.mkIf (cfg.wireless.enable && cfg.wireless.macAddress != null) {
      services.udev.extraRules = ''
        ACTION=="add", SUBSYSTEM=="net", SUBSYSTEMS=="pci", DRIVERS=="ath12k_pci", \
          RUN+="${pkgs.iproute2}/bin/ip link set dev $name address ${cfg.wireless.macAddress}"
      '';
    })

    (lib.mkIf (cfg.bluetooth.enable && cfg.bluetooth.macAddress != null) {
      services.udev.extraRules = ''
        ACTION=="add", SUBSYSTEM=="bluetooth", ENV{DEVTYPE}=="host", \
          ENV{DEVPATH}=="*/serial[0-9]*/serial[0-9]*/bluetooth/hci[0-9]*", \
          TAG+="systemd", ENV{SYSTEMD_WANTS}="hci-btaddress@%k.service"
      '';

      # udev fires this the moment the host device appears, which is before
      # the controller has finished initialising, hence upstream's sleep.
      # The instance name arrives as an argument rather than as a `%I` inside
      # the script: systemd expands specifiers in ExecStart, not in the body
      # of the script file NixOS generates.
      systemd.services."hci-btaddress@" = {
        description = "HCI bluetooth address fix";
        scriptArgs = "%I";
        script = ''
          sleep 5
          yes | ${pkgs.bluez}/bin/btmgmt -i "$1" public-addr "${cfg.bluetooth.macAddress}"
        '';
        serviceConfig.Type = "oneshot";
      };
    })
  ]);
}
