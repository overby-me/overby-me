{pkgs, ...}: {
  home.packages = with pkgs.pkgsUnstable; [
    acpi
    util-linux
    pciutils
    lshw
    usbutils
    solaar # Logitech Unifying Receiver
  ];
}
