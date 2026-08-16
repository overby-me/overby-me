{
  name = "64-UDEV-STORAGE";
  # Skips rather than passes: no multi-device storage topology in the VM
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
  # Upstream 64-UDEV-STORAGE requires extensive storage hardware setup
  # (multiple block devices, LVM, LUKS, MD, iSCSI) not present in the
  # nixosTest VM.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'oxidized-systemd: storage topology not configured in VM, skipping' >/skipped"
      echo "exit 77"
    } > TEST-64-UDEV-STORAGE.sh
    chmod +x TEST-64-UDEV-STORAGE.sh
  '';
}
