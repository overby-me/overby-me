{
  name = "09-REBOOT";
  # The journal subtest does `systemctl reboot` and re-runs across boots, so the
  # VM must actually restart (default -no-reboot would terminate QEMU on reboot).
  allowReboot = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal\\.sh$";
  };
}
