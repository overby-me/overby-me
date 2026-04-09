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
    # NixOS installs browse.html into the read-only Nix store, so the mv
    # that temporarily removes it will fail.  Skip only the browse.html
    # move/restore portion; keep the upload garbage, journal file-move,
    # and restore tests.
    sed -i '/^mv \/usr\/share\/systemd\/gatewayd\/browse\.html/,/^grep -qF.*title.*Journal/c\echo "SKIP: browse.html mv (read-only Nix store)"' TEST-04-JOURNAL.journal-gatewayd.sh
  '';
}
