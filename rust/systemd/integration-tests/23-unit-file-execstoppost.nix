{
  name = "23-UNIT-FILE";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.ExecStopPost\\.sh$";
  };
  # Restored 2026-07-27. This wrapper used to delete two whole sections from the
  # upstream script with perl -0pe: the Type=dbus section (dbus1.service ->
  # /run/dbus3) as "needs D-Bus", and the Type=notify section (notify1.sh ->
  # /run/notify2) as "needs READY=1 timeout handling". Both service types work
  # now, so deleting the assertions only hid whatever they would find. They run
  # again here.
  #
  # GREEN as of 2026-08-03, the EVENT-LOOP.md inc 3 expected flip. The long
  # arc, kept short: restoring the deleted sections found that ANY deferred
  # start failure skipped ExecStopPost=, and the in-place fix deadlocked
  # PID 1 (deferred_start_fail_cleanup held the table read guard plus the
  # state write guard while run_poststop waited on a helper underneath).
  # The dispatcher's poststop chains resolve exactly that: failures now run
  # ExecStopPost= initiate-only between brief guards and finalize after.
  # Getting every section green also fixed three real bugs: the transient
  # property loop dropped BusName= (Type=dbus transients started
  # unconfigured), a Type=dbus main exiting before its name appeared left a
  # stale parked wait armed, and a Type=notify main exiting before READY=1
  # deactivated cleanly instead of failing with upstream's protocol result.
  #
  # What remains is environment-only and does NOT reduce coverage: upstream
  # writes ExecStopPost='touch ...' with a bare command name, and NixOS has no
  # /usr/bin/touch, so the path is made absolute. The assertion itself is
  # untouched.
  patchScript = ''
    sed -i "s|ExecStopPost='touch |ExecStopPost='/run/current-system/sw/bin/touch |g" TEST-23-UNIT-FILE.ExecStopPost.sh
  '';
}
