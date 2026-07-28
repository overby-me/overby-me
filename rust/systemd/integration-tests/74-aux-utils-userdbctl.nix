{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.userdbctl\\.sh$";
  };
  # WHERE IT STOPS, measured 2026-07-28. Everything up to line ~224 passes,
  # including the whole -j/--json path and `userdbctl -F-`. It dies on
  #
  #     systemd-run -q -t --property 'SystemCallFilter=~open_tree' \
  #         id definitelynotarealuser
  #
  # which is piped into `grep 'no such user'`. The unit really runs and exits
  # (PID 1 logs REAP ... run-u...-id.service -> ServiceExited), but NOTHING
  # comes back on stdout, so the grep matches nothing. `-t` is --pty, so this
  # is pty output forwarding rather than anything userdb-specific, and it sits
  # next to the known-deep pty/script area. Note the failure surfaces in the
  # cleanup trap (userdel/groupdel), so the tail of the log names the cleanup,
  # not the defect.
}
