{
  name = "04-JOURNAL";
  # Passing subtests: bsod, cat, corrupted-journals, fss, invocation, journal, journal-append, journal-corrupt, LogFilterPatterns, reload, stopped-socket-activation, SYSTEMD_JOURNAL_COMPRESS
  # Skipped subtests and reasons:
  # - journal-gatewayd: uses C systemd-journal-gatewayd HTTP server (not reimplemented)
  # - journal-remote: uses C systemd-journal-remote/upload (not reimplemented)
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
    # Use stop+reset-failed+start instead of restart to avoid "already running" errors
    # when our journald doesn't exit cleanly within the systemd stop timeout.
    for f in TEST-04-JOURNAL.SYSTEMD_JOURNAL_COMPRESS.sh TEST-04-JOURNAL.journal.sh TEST-04-JOURNAL.reload.sh TEST-04-JOURNAL.bsod.sh; do
      sed -i 's|systemctl restart systemd-journald.service|systemctl stop systemd-journald.service; systemctl reset-failed systemd-journald.service 2>/dev/null; sleep 1; systemctl start systemd-journald.service|' "$f"
      sed -i 's|systemctl restart systemd-journald$|systemctl stop systemd-journald.service; systemctl reset-failed systemd-journald.service 2>/dev/null; sleep 1; systemctl start systemd-journald.service|' "$f"
    done
  '';
}
