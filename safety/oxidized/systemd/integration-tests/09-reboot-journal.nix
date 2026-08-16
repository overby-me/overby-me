{
  name = "09-REBOOT";
  # The journal subtest does `systemctl reboot` and re-runs across boots, so the
  # VM must actually restart. allowReboot (in-place QEMU reset) gets furthest:
  # boot 0 + boot 1 both boot healthy, only boot 1's backdoor console stays
  # silent (unsolved). useBootLoader was tried but panics early (oxidized-systemd
  # can't yet boot from a systemd-boot/EFI disk image — init exits before PID 1
  # logs). Keeping allowReboot as the least-broken state.
  allowReboot = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal\\.sh$";
  };
}
