# 07-PID1 .subgroup-kill subtest booted under SYSTEMD_RS_JOB_GRAPH=1
# (increment-4 A/B). Exercises systemctl kill --kill-subgroup / cgroup kill-whom
# under the flag.
{
  name = "07-PID1";
  jobGraph = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.subgroup-kill\\.sh$";
  };
}
