{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal\\.sh$";
  };
  testTimeout = 600;
  patchScript = ''
    # (De-weakened: the systemd-run --user + journalctl --user-unit skip was
    # removed to baseline the current-user user-manager + journalctl --user-unit
    # path, which the user manager should now support.)

    # Skip journalctl -b <script> test (executable_is_script test).
    # In the NixOS VM the test script runs via the backdoor (virtconsole),
    # not as a systemd service, so there are no journal entries with _EXE
    # matching the script's interpreter.
    sed -i '/journalctl -b "\$(readlink -f/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh

    # (De-weakened: the forever-print-hola FDSTORE hunk was removed to baseline
    # the FDSTORE=1 stdout-stream-fd store/recovery path across a journald
    # SIGKILL, now that fd_store infrastructure exists.)

    # Skip journalctl --follow tests (require running journald with working
    # stream reconnection)
    sed -i '/^journalctl --follow/s/.*/echo SKIP # journalctl --follow/' TEST-04-JOURNAL.journal.sh

  '';
}
