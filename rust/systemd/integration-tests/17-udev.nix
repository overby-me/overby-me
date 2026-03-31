{
  name = "17-UDEV";
  # Skip subtests that hang or depend on features unavailable in the NixOS test VM
  patchScript = ''
    # buffer-size: hangs waiting for udev monitor events in QEMU
    rm -f TEST-17-UDEV.buffer-size.sh
  '';
}
