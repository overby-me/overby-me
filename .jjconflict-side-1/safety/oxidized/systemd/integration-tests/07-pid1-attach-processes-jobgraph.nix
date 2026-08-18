# 07-PID1 .attach_processes subtest booted under SYSTEMD_RS_JOB_GRAPH=1
# (increment-4 A/B). Exercises runtime AttachProcessesToUnit under the flag.
{
  name = "07-PID1";
  jobGraph = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.attach_processes\\.sh$";
  };
}
