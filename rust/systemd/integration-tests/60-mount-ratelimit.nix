{
  name = "60-MOUNT-RATELIMIT";
  # De-skipped: the /proc/self/mountinfo-driven mount monitor (shared with
  # 10-MOUNT) is now implemented. Baselining un-skipped to find the real first
  # failure of the rate-limit recovery path before implementing it.
}
