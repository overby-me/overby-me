{
  name = "86-MULTI-PROFILE-UKI";
  # Skips rather than passes: the VM boots kernel+initrd directly, not a UKI
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
  # 86-MULTI-PROFILE-UKI requires an actual UKI boot (Unified Kernel Image)
  # with a stub binary; the NixOS test VM boots via legacy kernel + initrd.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: VM booted without UKI stub, skipping' >/skipped"
      echo "exit 77"
    } > TEST-86-MULTI-PROFILE-UKI.sh
    chmod +x TEST-86-MULTI-PROFILE-UKI.sh
  '';
}
