# 17-UDEV .sanity-check subtest booted under SYSTEMD_RS_JOB_GRAPH=1
# (increment-4 A/B). Udev-heavy boot closure: validates the drive's device
# completion scan on a test that stresses udev.
{
  name = "17-UDEV";
  jobGraph = true;
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.sanity-check\\.sh$";
  };
}
