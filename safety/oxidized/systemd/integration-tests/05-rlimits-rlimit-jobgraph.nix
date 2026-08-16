# 05-RLIMITS .rlimit subtest booted under SYSTEMD_RS_JOB_GRAPH=1 (increment-4
# A/B). Exercises rlimit exec-context under the flag (different boot closure).
{
  name = "05-RLIMITS";
  jobGraph = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.rlimit\\.sh$";
  };
}
