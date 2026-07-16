{
  name = "82-SOFTREBOOT";
  # Upstream 82-SOFTREBOOT tests the soft-reboot PID1 feature which is not
  # implemented by rust-systemd.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: soft-reboot not implemented, skipping' >/skipped"
      echo "exit 77"
    } > TEST-82-SOFTREBOOT.sh
    chmod +x TEST-82-SOFTREBOOT.sh
  '';
}
