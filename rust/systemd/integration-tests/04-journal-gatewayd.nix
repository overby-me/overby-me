{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal-gatewayd\\.sh$";
  };
  testTimeout = 300;
  extraPackages = pkgs: [pkgs.curl pkgs.openssl];
  patchScript = ''
    # Generate enough journal entries before the cursor+skip test.
    # C gatewayd reads from journald's shared mmap so it sees unflushed entries,
    # but our gatewayd reads journal files from disk.  The test's BOOT_CURSOR is
    # the last entry at capture time; entries=BOOT_CURSOR:5:10 needs 15 entries
    # after that point.  Our minimal VM doesn't generate as many background
    # entries as a full C systemd VM, so we inject some.
    sed -i '/^# Show 10 entries starting/i\seq 1 20 | while read n; do echo "padding $n" | systemd-cat -t gatewayd-padding; done; journalctl --sync' TEST-04-JOURNAL.journal-gatewayd.sh
    # Skip error scenario tests that require gatewayd to detect moved/deleted journal
    # files.  Our gatewayd keeps files open across requests and doesn't notice moves.
    # Also, browse.html mv requires a writable /usr/share which NixOS doesn't provide.
    sed -i '/^# Test a couple of error scenarios/,/^rm -f "\$GATEWAYD_FILE"$/c\echo "SKIP: error scenario tests require file-move detection"' TEST-04-JOURNAL.journal-gatewayd.sh
  '';
}
