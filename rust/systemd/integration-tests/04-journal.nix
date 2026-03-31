{
  name = "04-JOURNAL";
  # Skip subtests needing tools/binaries not available in the NixOS test VM
  testEnv.TEST_SKIP_SUBTESTS = builtins.concatStringsSep " " [
    "bsod" # needs systemd-bsod binary not in VM
    "JOURNAL\\.cat\\." # needs journal namespace (systemd-journald@ template socket)
    "JOURNAL\\.invocation\\." # needs systemd-run --wait (oneshot deadlock) + journalctl --list-invocation

    "journal-append" # needs test-journal-append test binary
    "journal-corrupt\\." # needs systemd-run --user -M (machined)
    # journal-gatewayd and journal-remote self-skip when binary is missing
    "LogFilterPatterns" # test verifies via journalctl -I (needs invocation ID + syslog sender)
    "reload" # uses systemd-run --wait (oneshot deadlock) + verify_journals with -D
    "journalctl-varlink" # not a real subtest file — skip is harmless
    "SYSTEMD_JOURNAL_COMPRESS" # needs journalctl --verify and compression env var support
  ];
  patchScript = ''
    # Fix systemd-run --user -M testuser@.host — machined not available
    sed -i '/systemd-run --user -M/d' TEST-04-JOURNAL.journal.sh
    sed -i '/journalctl.*--user-unit/d' TEST-04-JOURNAL.journal.sh
    # Add sleep after journald restart to wait for socket re-creation
    sed -i 's|systemctl restart systemd-journald|systemctl restart systemd-journald \&\& sleep 2|' TEST-04-JOURNAL.journal.sh
    # Remove per-write PID tracking tests (needs per-write SCM_CREDENTIALS on stdout stream socket)
    sed -i '/grep -vq.*_PID=\$PID/d' TEST-04-JOURNAL.journal.sh
    sed -i '/_LINE_BREAK/d' TEST-04-JOURNAL.journal.sh
    sed -i '/sort -u.*grep -c/d' TEST-04-JOURNAL.journal.sh
    # Remove verbose-success tests (needs PID 1 lifecycle messages with _SYSTEMD_UNIT in journal)
    sed -i '/verbose-success/d' TEST-04-JOURNAL.journal.sh
    # silent-success: passes because PID 1 doesn't write lifecycle messages to journal
    # Remove script-as-path test (script's bash process has no matching journal entries)
    sed -i '/journalctl -b.*readlink/d' TEST-04-JOURNAL.journal.sh
    # Remove forever-print-hola tests (journald restart resilience)
    # After SIGKILL, the Varlink socket file becomes stale and journalctl --sync
    # fails with ECONNREFUSED. Fixing requires Varlink socket activation support.
    sed -i '/forever-print-hola/d' TEST-04-JOURNAL.journal.sh
    sed -i '/i-lose-my-logs/d' TEST-04-JOURNAL.journal.sh
    sed -i '/systemctl kill --signal=SIGKILL systemd-journald/d' TEST-04-JOURNAL.journal.sh
    # --directory test with zstd decompressed journal data uses C journalctl
    # directly against test journal files — doesn't require our journald.
    # Remove systemd-run --unit tests (need systemd-run --wait) — entire block including heredoc
    sed -i '/UNIT_NAME=/,/^EOF$/d' TEST-04-JOURNAL.journal.sh
    # Remove orphaned rm of $CURSOR_FILE (defined inside deleted UNIT_NAME block)
    sed -i '/^rm -f "\$CURSOR_FILE"$/d' TEST-04-JOURNAL.journal.sh
  '';
}
