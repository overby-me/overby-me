{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.id128\\.sh$";
  };
  # No patch needed: systemd-id128 rejects an over-long (65-char) id with a
  # non-zero exit, so the upstream `(! systemd-id128 show <65 zeros>)` negative
  # case passes as-is. (The skip that deleted that line was stale.)
  patchScript = "";
}
