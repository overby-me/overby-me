# Fairphone 5 bootmac module: deterministic WiFi/Bluetooth MAC address
# generation from the device's serial number.
#
# Without this, both WiFi and Bluetooth will use randomly generated MAC
# addresses that change on every reboot, causing issues with network
# identification and Bluetooth pairing.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.nixos-fairphone-fp5.bootmac;
in {
  options.nixos-fairphone-fp5.bootmac = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Enable bootmac to configure deterministic MAC addresses at boot.";
    };

    bluetooth = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Enable Bluetooth MAC address configuration.";
      };

      interface = lib.mkOption {
        type = lib.types.str;
        default = "hci0";
        description = "Name of the Bluetooth interface to configure.";
      };
    };

    wifi = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Enable WiFi MAC address configuration.";
      };

      interface = lib.mkOption {
        type = lib.types.str;
        default = "wlan0";
        description = "Name of the WiFi interface to configure.";
      };
    };

    macPrefix = lib.mkOption {
      type = lib.types.str;
      default = "0200";
      description = ''
        MAC address prefix. Default "0200" = locally administered unicast.
      '';
    };

    timeout = lib.mkOption {
      type = lib.types.int;
      default = 5;
      description = "Timeout in seconds for setting MAC addresses.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [pkgs.bootmac];

    # Set Bluetooth MAC address before bluetoothd starts.
    systemd.services.bootmac-bluetooth = lib.mkIf cfg.bluetooth.enable {
      description = "Set Bluetooth MAC address";

      after = ["sys-subsystem-bluetooth-devices-${cfg.bluetooth.interface}.device"];
      requires = ["sys-subsystem-bluetooth-devices-${cfg.bluetooth.interface}.device"];

      before = ["bluetooth.service"];
      wantedBy = ["bluetooth.service"];

      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${pkgs.bootmac}/bin/bootmac --bluetooth-if ${cfg.bluetooth.interface} --prefix ${cfg.macPrefix}";
        Environment = "BT_TIMEOUT=${toString cfg.timeout}";
      };
    };

    # Set WiFi MAC address before network starts.
    systemd.services.bootmac-wifi = lib.mkIf cfg.wifi.enable {
      description = "Set WiFi MAC address";

      after = ["sys-subsystem-net-devices-${cfg.wifi.interface}.device"];
      requires = ["sys-subsystem-net-devices-${cfg.wifi.interface}.device"];

      before = ["network-pre.target"];
      wantedBy = ["network-pre.target"];

      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${pkgs.bootmac}/bin/bootmac --wlan-if ${cfg.wifi.interface} --prefix ${cfg.macPrefix}";
        Environment = "WLAN_TIMEOUT=${toString cfg.timeout}";
      };
    };

    # Ensure bluez is available for btmgmt command (required for Bluetooth).
    services.blueman.enable = lib.mkIf cfg.bluetooth.enable true;
  };
}
