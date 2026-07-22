{
  name = "09-REBOOT";
  # The journal subtest does `systemctl reboot` and re-runs across boots, so the
  # VM must actually restart. useBootLoader boots via a real bootloader/disk so a
  # firmware reboot re-runs the whole boot (re-establishing the driver's backdoor
  # console); it implies allow_reboot. allowReboot alone (in-place QEMU reset)
  # leaves the second boot's backdoor console silent.
  useBootLoader = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal\\.sh$";
  };
}
