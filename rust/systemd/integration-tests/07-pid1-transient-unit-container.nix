{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.transient-unit-container\\.sh$";
  };
  # WHERE IT STOPS, measured 2026-07-28. The test runs
  #
  #     systemd-run --unit TEST-07-PID1.transient-unit-container.service \
  #         --wait -p RootDirectory=/tmp/TEST-07-PID1.transient-unit-... ...
  #
  # and then reads back the service's output file, asserting it contains
  # "Test service is running". The file is EMPTY, so the assertion sees the
  # empty string. The service is created and waited on; what does not happen is
  # it producing output from inside the RootDirectory= tree.
  #
  # The failure surfaces in file_write_cleanup, so the tail of the log names
  # the cleanup rather than the defect; the real assertion is about ten traced
  # lines further back.
}
