{
  name = "17-UDEV";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.watch\\.sh$";
  };
  # WHERE IT STOPS, measured 2026-07-28. After restarting systemd-udevd it
  # asserts, via journalctl --invocation=0 --grep, that udevd logged
  #
  #     Received inotify fd (N) from service manager.
  #
  # journalctl --invocation IS implemented, so that is not the gap: the string
  # appears NOWHERE in crates/, because udevd never receives its inotify fd
  # from the service manager in the first place. Upstream passes it through the
  # fd store so device watches survive a udevd restart, which means work on
  # both sides, PID 1 keeping the fd and udevd accepting it. Not a udevadm or
  # journalctl gap.
}
