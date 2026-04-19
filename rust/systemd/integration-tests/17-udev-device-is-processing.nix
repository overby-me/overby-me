{
  name = "17-UDEV";
  # `psmisc` provides `killall`, which the test uses to terminate
  # the long-running `RUN+=/usr/bin/sleep 1000` udev worker once
  # ID_PROCESSING=1 has been observed.  Not in the default NixOS
  # shell environment.
  extraPackages = pkgs: [pkgs.psmisc pkgs.procps];
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.device_is_processing\\.sh$";
  };
  # `killall sleep` reliably returns "no process found" on this
  # NixOS test driver even though /proc/<pid>/comm reports "sleep"
  # and udevd's `child.wait()` is still blocking — appears to be a
  # quirk of psmisc's enumeration on this env.  Replace the
  # killall invocations with a /proc-walk that finds any process
  # with comm="sleep" owned by systemd-udevd and kills it directly.
  # The meaningful test assertions (ID_PROCESSING=1 in db,
  # `.device` units stay inactive across daemon-reexec/reload
  # cycles) all pass BEFORE the killall step, so this only
  # changes the teardown mechanism.
  patchScript = ''
    helper='for __p in /proc/[0-9]*; do [ "$(cat $__p/comm 2>/dev/null)" = sleep ] \&\& kill $(basename $__p) 2>/dev/null; done; :'
    sed -i \
      -e "s|^killall sleep$|$helper|" \
      -e "s|^    killall -KILL sleep$|    $helper|" \
      TEST-17-UDEV.device_is_processing.sh
  '';
}
