# 23-UNIT-FILE .exec-command-ex subtest booted under SYSTEMD_RS_JOB_GRAPH=1
# (increment-4 A/B). Exercises ExecCommandEx unit-file parsing under the flag.
{
  name = "23-UNIT-FILE";
  jobGraph = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec-command-ex\\.sh$";
  };
}
