{
  name = "17-UDEV";
  # `psmisc` provides `killall`, which the test uses to terminate
  # the long-running `RUN+=/usr/bin/sleep 1000` udev worker once
  # ID_PROCESSING=1 has been observed.  Not in the default NixOS
  # shell environment.
  extraPackages = pkgs: [pkgs.psmisc];
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.device_is_processing\\.sh$";
  };
}
