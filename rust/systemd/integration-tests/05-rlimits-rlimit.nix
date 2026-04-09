{
  name = "05-RLIMITS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.rlimit\\.sh$";
  };
  patchScript = ''
    sed -i 's/systemd-run --wait -t/systemd-run --wait --pipe/' TEST-05-RLIMITS.rlimit.sh
  '';
}
