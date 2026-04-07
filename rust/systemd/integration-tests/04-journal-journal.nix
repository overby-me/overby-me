{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal\\.sh$";
  };
  testTimeout = 600;
  patchScript = ''
    # Debug: dump _EXE values and stdout debug log before the script-path match test
    sed -i '/journalctl -b "$(readlink -f/i\echo "=== _EXE values ===" ; journalctl --field _EXE 2>\&1 || true ; echo "=== END _EXE ===" ; echo "=== STDOUT DEBUG ===" ; cat /tmp/journald-stdout-debug.log 2>\&1 | tail -100 || true ; echo "=== END STDOUT DEBUG ==="' TEST-04-JOURNAL.journal.sh
  '';
}
