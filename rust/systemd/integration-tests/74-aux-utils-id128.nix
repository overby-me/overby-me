{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.id128\\.sh$";
  };
  patchScript = ''
    sed -i "/printf.*%0.s0.*{0..64}/d" TEST-74-AUX-UTILS.id128.sh
  '';
}
