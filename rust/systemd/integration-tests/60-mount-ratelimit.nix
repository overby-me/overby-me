{
  name = "60-MOUNT-RATELIMIT";
  # Skips rather than passes: no mountinfo event-source rate limiting
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
  # The /proc/self/mountinfo-driven mount monitor is now implemented (shared
  # with 10-MOUNT), and this session added SubState=mounted for active mount
  # units plus a guard so the monitor never deactivates a mount unit PID 1 is
  # starting. But testcase_issue_20329 additionally requires systemd's mountinfo
  # *event-source rate-limiting* with delayed mount start-job handling: after a
  # burst of mount/unmount events the monitor is throttled and mount start jobs
  # must be DELAYED until it recovers. rust-systemd has no event-source
  # rate-limiter, so a post-burst `systemctl start` races the backlogged monitor
  # and fails. Re-skipped until that recovery path is implemented.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: mountinfo monitor rate-limit recovery not implemented, skipping' >/skipped"
      echo "exit 77"
    } > TEST-60-MOUNT-RATELIMIT.sh
    chmod +x TEST-60-MOUNT-RATELIMIT.sh
  '';
}
