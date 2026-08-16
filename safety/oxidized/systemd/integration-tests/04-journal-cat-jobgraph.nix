# 04-JOURNAL .cat subtest booted under SYSTEMD_RS_JOB_GRAPH=1 (increment-4 A/B).
# Exercises the job-graph boot path with the journal subsystem up (a different
# boot closure than 01-basic / 07-pid1).
{
  name = "04-JOURNAL";
  jobGraph = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cat\\.sh$";
  };
  testTimeout = 300;
}
