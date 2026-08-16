{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.private-network\\.sh$";
  };
  patchScript = ''    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
  '';
}
