{
  name = "07-PID1";
  extraPackages = pkgs: [pkgs.socat];
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.socket-defer\\.sh$";
  };
}
