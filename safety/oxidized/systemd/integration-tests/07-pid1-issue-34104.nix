{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.issue-34104\\.sh$";
  };
  patchScript = ''    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
  '';
}
