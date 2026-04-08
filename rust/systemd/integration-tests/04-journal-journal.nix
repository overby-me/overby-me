{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal\\.sh$";
  };
  testTimeout = 600;
  patchScript = ''
    # Skip systemd-run --user (user session not fully supported)
    sed -i '/^systemd-run --user/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh
    # Also skip the journalctl --user-unit check that follows it
    sed -i '/^journalctl -b -n 1 -r --user-unit/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh

    # Skip journalctl -b <script> test (executable_is_script test).
    # In the NixOS VM the test script runs via the backdoor (virtconsole),
    # not as a systemd service, so there are no journal entries with _EXE
    # matching the script's interpreter.
    sed -i '/journalctl -b "\$(readlink -f/s/.*/echo SKIP/' TEST-04-JOURNAL.journal.sh

  '';
}
