{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.machine\\-id\\-setup\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --state=failed/,/test ! -s/d' TEST-74-AUX-UTILS.machine-id-setup.sh
  '';
}
