# 07-PID1 .socket-defer subtest booted under SYSTEMD_RS_JOB_GRAPH=1 (increment-4
# A/B). Exercises the drive on a boot closure containing .socket units (a
# process-less unit type, like .device: verifies the completion scan retires
# them rather than hanging the closure).
{
  name = "07-PID1";
  jobGraph = true;
  extraPackages = pkgs: [pkgs.socat];
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.socket-defer\\.sh$";
  };
}
