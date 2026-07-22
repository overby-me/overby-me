{
  name = "82-SOFTREBOOT";
  # De-skipped to baseline the real first failure. Upstream 82-SOFTREBOOT tests
  # the soft-reboot PID1 feature (systemctl soft-reboot: userspace-only restart
  # that stops units, re-execs PID1, optionally switches to /run/nextroot, and
  # restarts, without a kernel reboot).
}
