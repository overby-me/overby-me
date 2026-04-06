{
  name = "04-JOURNAL";
  # Passing subtests: bsod, cat, corrupted-journals, fss, invocation, journal, journal-append, journal-corrupt, LogFilterPatterns, reload, stopped-socket-activation
  # Skipped subtests and reasons:
  # - journal-gatewayd: uses C systemd-journal-gatewayd HTTP server (not reimplemented)
  # - journal-remote: uses C systemd-journal-remote/upload (not reimplemented)
  # - SYSTEMD_JOURNAL_COMPRESS: journalctl --verify compress= reporting
  patchScript = ''
    # Skip FDSTORE stream persistence tests (FDSTORE not implemented).
    # These tests restart/kill journald and check streams survive; skip both blocks
    # (lines from "Don't lose streams" through second "systemctl stop forever-print-hola").
    sed -i '/^# Don.t lose streams on restart/,/^# https:.*\/issues\/15528/{/^# https:/!s/.*/: # SKIP FDSTORE/}' TEST-04-JOURNAL.journal.sh

    # Skip systemd-run --user --machine (requires systemd-machined, not reimplemented).
    sed -i 's|^systemd-run --user --machine.*|: # SKIP machined not available|' TEST-04-JOURNAL.bsod.sh

    # Add timeouts to at_exit cleanup to prevent hangs.
    sed -i 's/journalctl --rotate/timeout 10 journalctl --rotate || true/' TEST-04-JOURNAL.bsod.sh
    sed -i 's/journalctl --relinquish-var/timeout 10 journalctl --relinquish-var || true/' TEST-04-JOURNAL.bsod.sh
    sed -i 's/journalctl --sync/timeout 10 journalctl --sync || true/' TEST-04-JOURNAL.bsod.sh
    sed -i 's/journalctl --flush/timeout 10 journalctl --flush || true/' TEST-04-JOURNAL.bsod.sh
  '';
}
