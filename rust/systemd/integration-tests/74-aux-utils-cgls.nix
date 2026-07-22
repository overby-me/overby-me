{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cgls\\.sh$";
  };
  # De-weakened: the previous patchScript deleted the `systemd-run --user
  # --wait --pipe -M testuser` line and its `--user-unit` check. The user
  # manager works now, so baseline un-skipped to find the real first failure of
  # the machine-routed user-service path.
}
