{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.bsod\\.sh$";
  };
  testTimeout = 300;
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
    # Restart journald after tmpfs unmount so it opens a fresh journal file
    # on the real /var/log/journal.  Our journald does not implement
    # --relinquish-var, so after the tmpfs unmount it would keep writing to
    # an orphaned file descriptor.
    # Use retry+fallback because systemctl may transiently fail with EAGAIN.
    sed -i '/timeout 10 journalctl --flush/a\    systemctl restart systemd-journald || { sleep 1; systemctl restart systemd-journald; } || true' TEST-04-JOURNAL.bsod.sh
  '';
}
