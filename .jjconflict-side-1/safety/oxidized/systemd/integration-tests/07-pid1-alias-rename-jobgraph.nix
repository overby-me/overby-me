# 07-PID1 .alias-rename subtest booted under SYSTEMD_RS_JOB_GRAPH=1 (increment-4
# A/B). Stresses the job-graph boot path across daemon-reload + reexec (the
# subtest renames a running unit's fragment to an alias symlink and reloads).
{
  name = "07-PID1";
  jobGraph = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.alias-rename\\.sh$";
  };
}
