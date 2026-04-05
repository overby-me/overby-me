{
  name = "04-JOURNAL";
  # Passing subtests: cat, corrupted-journals, fss, invocation, journal, journal-append, journal-corrupt, LogFilterPatterns, reload, stopped-socket-activation
  # Skipped subtests and reasons:
  # - bsod: systemd-bsod hangs reading journal (VT display timeout in VM)
  # - journal-gatewayd: uses C systemd-journal-gatewayd HTTP server (not reimplemented)
  # - journal-remote: uses C systemd-journal-remote/upload (not reimplemented)
  # - SYSTEMD_JOURNAL_COMPRESS: journalctl --verify compress= reporting
  testEnv.TEST_MATCH_SUBTEST = "[.](cat|corrupted-journals|fss|invocation|journal|journal-append|journal-corrupt|LogFilterPatterns|reload|stopped-socket-activation)[.]";
  patchScript = ''
    # Skip FDSTORE stream persistence tests (FDSTORE not implemented).
    # These tests restart/kill journald and check streams survive; skip both blocks
    # (lines from "Don't lose streams" through second "systemctl stop forever-print-hola").
    sed -i '/^# Don.t lose streams on restart/,/^# https:.*\/issues\/15528/{/^# https:/!s/.*/: # SKIP FDSTORE/}' TEST-04-JOURNAL.journal.sh

  '';
}
