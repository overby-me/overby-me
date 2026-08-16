{
  name = "17-UDEV";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.SYSTEMD_WANTS\\.sh$";
  };
}
