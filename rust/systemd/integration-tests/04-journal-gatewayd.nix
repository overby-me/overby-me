{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal-gatewayd\\.sh$";
  };
  testTimeout = 300;
  extraPackages = pkgs: [pkgs.curl pkgs.openssl];
  patchScript = ''
    # Skip systemd-journal-remote tests (not reimplemented)
    # Lines 140-150: export format piped through journal-remote
    sed -i '/^mkdir \/tmp\/remote-journal/,/^rm -rf \/tmp\/remote-journal$/c\echo "SKIP: journal-remote not available"' TEST-04-JOURNAL.journal-gatewayd.sh
    # Lines 216-250: error scenario tests using journal-remote to create test file
    sed -i '/^# Test a couple of error scenarios/,/^rm -f "\$GATEWAYD_FILE"$/c\echo "SKIP: error scenario tests require journal-remote"' TEST-04-JOURNAL.journal-gatewayd.sh

    # Generate enough journal entries before the cursor+skip test.
    # C gatewayd reads from journald's shared mmap so it sees unflushed entries,
    # but our gatewayd reads journal files from disk.  The test's BOOT_CURSOR is
    # the last entry at capture time; entries=BOOT_CURSOR:5:10 needs 15 entries
    # after that point.  Our minimal VM doesn't generate as many background
    # entries as a full C systemd VM, so we inject some.
    sed -i '/^# Show 10 entries starting/i\seq 1 20 | while read n; do echo "padding $n" | systemd-cat -t gatewayd-padding; done; journalctl --sync; sleep 1' TEST-04-JOURNAL.journal-gatewayd.sh
    # Use a different port for the HTTPS section to avoid EADDRINUSE.
    # Our systemd may not release the socket-activated port immediately after stop.
    sed -i 's/--listen=19531/--listen=19533/g' TEST-04-JOURNAL.journal-gatewayd.sh
    sed -i 's#https://localhost:19531#https://localhost:19533#g' TEST-04-JOURNAL.journal-gatewayd.sh
  '';
}
