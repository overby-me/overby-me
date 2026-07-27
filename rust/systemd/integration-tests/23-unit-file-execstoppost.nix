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
  # RESTORING THEM IMMEDIATELY FOUND A REAL BUG, which is what a deleted
  # assertion costs you. The Type=dbus case fails at `test -f /run/dbus1`.
  # Upstream starts a Type=dbus service whose command is a busctl RequestName
  # that exits straight away, wraps it in `|| :` so the exit status cannot
  # matter, and asserts only that ExecStopPost= ran. rust never runs it.
  #
  # ROOT CAUSE, located: the bus-name wait resolves on the DEFERRED start path,
  # and activate.rs deferred_start_fail_cleanup() — the shared cleanup for start
  # timeout, dbus-name timeout, exec confirmation failure and a failing forking
  # parent — kills the processes and marks the unit failed without ever running
  # ExecStopPost=. It is not dbus-specific: ANY service whose start fails on a
  # deferred path skips its ExecStopPost=.
  #
  # THE OBVIOUS FIX DEADLOCKS PID 1, so it is NOT applied. Calling run_poststop
  # from inside that function wedged the VM at forking1.service with the clock
  # frozen: deferred_start_fail_cleanup holds BOTH the RuntimeInfo read guard
  # and the service state write guard, and run_poststop waits on a helper
  # underneath them. That is exactly the hazard activate.rs already flags at the
  # ExecStartPost failure path ("this holds the RuntimeInfo read guard + state
  # write lock across the bounded poststart helper wait; taking helper waits
  # fully off the locks is docs/ARCHITECTURE.md invariant I1"). A real fix has to
  # run the ExecStopPost commands AFTER dropping both guards, which is the same
  # lock-decoupling work tracked for activate.rs generally — not a local edit.
  #
  # What remains is environment-only and does NOT reduce coverage: upstream
  # writes ExecStopPost='touch ...' with a bare command name, and NixOS has no
  # /usr/bin/touch, so the path is made absolute. The assertion itself is
  # untouched.
  patchScript = ''
    sed -i "s|ExecStopPost='touch |ExecStopPost='/run/current-system/sw/bin/touch |g" TEST-23-UNIT-FILE.ExecStopPost.sh
  '';
}
