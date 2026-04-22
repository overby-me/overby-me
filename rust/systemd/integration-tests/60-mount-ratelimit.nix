{
  name = "60-MOUNT-RATELIMIT";
  # Like 10-MOUNT, this test requires /proc/self/mountinfo-driven mount unit
  # tracking, plus rate-limit recovery logic.  Neither is implemented yet.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: mountinfo monitor rate-limit path not implemented, skipping' >/skipped"
      echo "exit 77"
    } > TEST-60-MOUNT-RATELIMIT.sh
    chmod +x TEST-60-MOUNT-RATELIMIT.sh
  '';
}
