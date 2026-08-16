{
  name = "74-AUX-UTILS";
  extraPackages = pkgs: [pkgs.socat];
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.socket\\-activate\\.sh$";
  };
}
