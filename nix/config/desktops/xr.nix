{pkgs, ...}: {
  environment.systemPackages = with pkgs; [
    monado
    stardust-xr-server
    stardust-xr-kiara
    non-spatial-input
    #flatland
    #weston
  ];
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
