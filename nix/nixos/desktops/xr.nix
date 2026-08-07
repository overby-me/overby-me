{pkgs, ...}: {
  environment.systemPackages = with pkgs; [
    monado
    stardust-xr-server
    non-spatial-input
    #flatland
    #weston
  ];
  # stardust-xr-kiara was dropped from nixpkgs on 2026-07-04, "no longer
  # compatible with the latest versions of the Stardust XR server", and the
  # alias now throws.  Surviving clients if a replacement is wanted:
  # stardust-xr-protostar, -phobetor, -atmosphere, -gravity, -sphereland.
  services.udev.extraRules = ''
    # XReal

    # Rule for USB devices
    SUBSYSTEM=="usb", ACTION=="add", ATTR{idVendor}=="3318", ATTR{idProduct}=="0424|0428|0432", MODE="0666"

    # Rule for Input devices (such as eventX)
    SUBSYSTEM=="input", KERNEL=="event[0-9]*", ATTRS{idVendor}=="3318", ATTRS{idProduct}=="0424|0428|0432", MODE="0666"

    # Rule for Sound devices (pcmCxDx and controlCx)
    SUBSYSTEM=="sound", KERNEL=="pcmC[0-9]D[0-9]p", ATTRS{idVendor}=="3318", ATTRS{idProduct}=="0424|0428|0432", MODE="0666"
    SUBSYSTEM=="sound", KERNEL=="controlC[0-9]", ATTRS{idVendor}=="3318", ATTRS{idProduct}=="0424|0428|0432", MODE="0666"

    # Rule for HID Devices (hidraw)
    SUBSYSTEM=="hidraw", KERNEL=="hidraw[0-9]*", ATTRS{idVendor}=="3318", ATTRS{idProduct}=="0424|0428|0432", MODE="0666"

    # Rule for HID Devices (hiddev)
    KERNEL=="hiddev[0-9]*", SUBSYSTEM=="usb", ATTRS{idVendor}=="3318", ATTRS{idProduct}=="0424|0428|0432", MODE="0666"
  '';

  # ── Home-Manager ──────────────────────────────────────────────────────────
  home-manager.sharedModules = [
    {
      xdg.configFile."openxr/1/active_runtime.json".source = "${pkgs.monado}/share/openxr/1/openxr_monado.json";
    }
  ];
}
