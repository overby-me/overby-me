{
  name = "04-JOURNAL";
  # Passing subtests: bsod, cat, corrupted-journals, fss, invocation, journal, journal-append, journal-corrupt, LogFilterPatterns, reload, stopped-socket-activation, SYSTEMD_JOURNAL_COMPRESS
  # Skipped subtests and reasons:
  # - journal-gatewayd: uses C systemd-journal-gatewayd HTTP server (not reimplemented)
  # - journal-remote: uses C systemd-journal-remote/upload (not reimplemented)
  patchScript = ''
    # Skip FDSTORE stream persistence tests (FDSTORE not implemented).
    # These tests restart/kill journald and check streams survive; skip both blocks
    # (lines from "Don't lose streams" through second "systemctl stop forever-print-hola").
    sed -i '/^# Don.t lose streams on restart/,/^# https:.*\/issues\/15528/{/^# https:/!s/.*/: # SKIP FDSTORE/}' TEST-04-JOURNAL.journal.sh

    # Skip systemd-run --user --machine (requires systemd-machined, not reimplemented).
    sed -i 's|^systemd-run --user --machine.*|: # SKIP machined not available|' TEST-04-JOURNAL.bsod.sh



  '';
}
