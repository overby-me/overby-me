{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.type-exec\\.sh$";
  };
  patchScript = ''
    # type-exec: remove busctl section (issue #20933, needs D-Bus)
    perl -i -0pe 's/# For issue #20933.*//s' TEST-23-UNIT-FILE.type-exec.sh
  '';
}
