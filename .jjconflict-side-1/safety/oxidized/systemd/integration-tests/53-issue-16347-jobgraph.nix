# 53-TIMER .issue-16347 subtest booted under SYSTEMD_RS_JOB_GRAPH=1
# (increment-4 A/B). Exercises the job-graph boot path with .timer units.
{
  name = "53-TIMER";
  jobGraph = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.issue-16347\\.sh$";
  };
}
