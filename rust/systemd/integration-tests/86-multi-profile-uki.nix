{
  name = "86-MULTI-PROFILE-UKI";
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
