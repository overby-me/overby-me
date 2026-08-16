# 07-PID1 .DeferReactivation subtest booted under SYSTEMD_RS_JOB_GRAPH=1
# (increment-4 A/B). Exercises deferred-start completion (the area the earlier
# 03-jobs stall lived in) under the drive.
{
  name = "07-PID1";
  jobGraph = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.DeferReactivation\\.sh$";
  };
}
