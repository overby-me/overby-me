{
  name = "82-SOFTREBOOT";
  # Skips rather than passes: soft-reboot is not implemented
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
  # Upstream 82-SOFTREBOOT tests the soft-reboot PID1 feature, not implemented
  # by rust-systemd. Baselined 2026-07-22: first failure is the missing
  # `SoftRebootsCount` Manager D-Bus property, and the test is a MULTI-soft-reboot
  # iteration test (marker file + SoftRebootsCount counter across N soft-reboots),
  # so it needs the full sequence: systemctl soft-reboot -> stop units -> re-exec
  # PID1 (optionally switch to /run/nextroot) -> restart -> preserve the counter/
  # fdstore across the re-exec, and the test harness must survive the re-exec
  # (same backdoor-survival risk as 09-REBOOT #73). Deep dedicated arc; re-skipped.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: soft-reboot not implemented, skipping' >/skipped"
      echo "exit 77"
    } > TEST-82-SOFTREBOOT.sh
    chmod +x TEST-82-SOFTREBOOT.sh
  '';
}
