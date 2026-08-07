# 81-GENERATORS .debug-generator subtest booted under SYSTEMD_RS_JOB_GRAPH=1
# (increment-4 A/B). Boots via the job graph with generators active.
{
  name = "81-GENERATORS";
  jobGraph = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.debug-generator\\.sh$";
  };
}
