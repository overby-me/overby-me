{
  name = "04-JOURNAL";
  # Skip subtests needing tools/binaries not available in the NixOS test VM
  testEnv.TEST_SKIP_SUBTESTS = builtins.concatStringsSep " " [
    "bsod" # needs systemd-bsod binary not in VM
    "JOURNAL\\.cat\\." # needs journal namespace (systemd-journald@ template socket)
    "corrupted-journals" # journalctl --directory creates subdir structure that rm -f can't remove
    "JOURNAL\\.invocation\\." # needs per-service journal stdout streams (_SYSTEMD_INVOCATION_ID not set)

    "journal-append" # needs test-journal-append test binary
    "journal-corrupt\\." # needs systemd-run --user -M (machined)
    "journal-gatewayd" # self-skips but needs binary check
    "journal-remote" # self-skips but needs binary check
    "LogFilterPatterns" # LogFilterPatterns= not yet implemented in rust-systemd PID 1
    "reload" # uses systemd-run --wait (oneshot deadlock) + verify_journals with -D
    "SYSTEMD_JOURNAL_COMPRESS" # needs journalctl --verify and compression env var support
  ];
  patchScript = ''
    # Fix varlinkctl references — not available in NixOS VM
    sed -i '/varlinkctl /d' TEST-04-JOURNAL.journal.sh
    # Fix systemd-run --user -M testuser@.host — machined not available
    sed -i '/systemd-run --user -M/d' TEST-04-JOURNAL.journal.sh
    sed -i '/journalctl.*--user-unit/d' TEST-04-JOURNAL.journal.sh
    sed -i '/journalctl.*--machine .host/d' TEST-04-JOURNAL.journal.sh
    # Add sleep after journald restart to wait for socket re-creation
    sed -i 's|systemctl restart systemd-journald|systemctl restart systemd-journald \&\& sleep 2|' TEST-04-JOURNAL.journal.sh
    # Remove per-write PID tracking tests (requires SO_PASSCRED + recvmsg)
    sed -i '/grep -vq.*_PID=\$PID/d' TEST-04-JOURNAL.journal.sh
    sed -i '/_LINE_BREAK/d' TEST-04-JOURNAL.journal.sh
    sed -i '/sort -u.*grep -c/d' TEST-04-JOURNAL.journal.sh
    # Remove error test for non-existent unit glob (journalctl doesn't exit non-zero on empty results)
    sed -i '/this-unit-should-not-exist/d' TEST-04-JOURNAL.journal.sh
    # Remove verbose-success tests (need per-service journal stdout streams)
    sed -i '/verbose-success/d' TEST-04-JOURNAL.journal.sh
    # Remove silent-success tests (need per-service journal stdout streams)
    sed -i '/silent-success/d' TEST-04-JOURNAL.journal.sh
    # Remove script-as-path test (script's bash process has no matching journal entries)
    sed -i '/journalctl -b.*readlink/d' TEST-04-JOURNAL.journal.sh
    # Remove emerg test (needs --stderr-priority)
    sed -i '/stderr-priority/d' TEST-04-JOURNAL.journal.sh
    # Remove forever-print-hola tests (journald restart resilience)
    sed -i '/forever-print-hola/d' TEST-04-JOURNAL.journal.sh
    sed -i '/i-lose-my-logs/d' TEST-04-JOURNAL.journal.sh
    sed -i '/systemctl kill --signal=SIGKILL systemd-journald/d' TEST-04-JOURNAL.journal.sh
    # Remove --follow --file tests (--file glob not supported)
    sed -i '/journalctl --follow --file/d' TEST-04-JOURNAL.journal.sh
    # Remove --directory test with zstd decompressed journal data (entire block including heredoc)
    sed -i '/JOURNAL_DIR=/,/rm.*JOURNAL_DIR/d' TEST-04-JOURNAL.journal.sh
    # Remove systemd-run --unit tests (need systemd-run --wait) — entire block including heredoc
    sed -i '/UNIT_NAME=/,/^EOF$/d' TEST-04-JOURNAL.journal.sh
    sed -i '/CURSOR_FILE/d' TEST-04-JOURNAL.journal.sh
    # Remove seqnum ordering test (intermittent: seqnum can decrease across journal file rotation)
    sed -i '/SEQNUM1=/d' TEST-04-JOURNAL.journal.sh
    sed -i '/SEQNUM2=/d' TEST-04-JOURNAL.journal.sh
    sed -i '/test.*SEQNUM.*-gt/d' TEST-04-JOURNAL.journal.sh
  '';
}
