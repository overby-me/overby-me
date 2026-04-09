{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cgls\\.sh$";
  };
  patchScript = ''
    sed -i '/systemd-run --user --wait --pipe -M testuser/d' TEST-74-AUX-UTILS.cgls.sh
    sed -i '/--user-unit/d' TEST-74-AUX-UTILS.cgls.sh
  '';
}
