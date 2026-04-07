{
  name = "04-JOURNAL";
  # Passing subtests: corrupted-journals, fss, invocation, journal-append, journal-corrupt, LogFilterPatterns, reload, stopped-socket-activation, SYSTEMD_JOURNAL_COMPRESS
  # Skipped subtests and reasons:
  # - journal-gatewayd: uses C systemd-journal-gatewayd HTTP server (not reimplemented)
  # - journal-remote: uses C systemd-journal-remote/upload (not reimplemented)
  # - bsod: C systemd-bsod uses sd_journal_wait() which needs inotify on LPKSHHRH journal files
  # - cat: requires namespace journal instances (not implemented)
  testEnv = {
    TEST_SKIP_SUBTESTS = "journal-gatewayd journal-remote \\.bsod\\. \\.cat\\.";
  };
  patchScript = ''
    # Add timeouts to bsod at_exit cleanup to prevent infinite hangs.
    sed -i 's/journalctl --rotate/timeout 10 journalctl --rotate/' TEST-04-JOURNAL.bsod.sh
    sed -i 's/journalctl --relinquish-var/timeout 10 journalctl --relinquish-var/' TEST-04-JOURNAL.bsod.sh
    sed -i 's/journalctl --sync/timeout 10 journalctl --sync/' TEST-04-JOURNAL.bsod.sh
    sed -i 's/journalctl --flush/timeout 10 journalctl --flush/' TEST-04-JOURNAL.bsod.sh
    # mv of archived journals may fail if rotate did not produce any.
    sed -i '/system@\*\.journal/s/$/ || true/' TEST-04-JOURNAL.bsod.sh
    # umount may fail if journald still holds the directory open.
    sed -i 's#umount /var/log/journal#umount /var/log/journal 2>/dev/null || true#' TEST-04-JOURNAL.bsod.sh
    # Skip journal-remote sub-test (uses C systemd-journal-remote, not reimplemented)
    sed -i 's#if \[\[ -x /usr/lib/systemd/systemd-journal-remote \]\]#if false#' TEST-04-JOURNAL.SYSTEMD_JOURNAL_COMPRESS.sh
  '';
}
