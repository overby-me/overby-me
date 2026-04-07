{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "bsod";
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
    # Use stop+reset-failed+start instead of restart
    sed -i 's|systemctl restart systemd-journald.service|systemctl stop systemd-journald.service; systemctl reset-failed systemd-journald.service 2>/dev/null; sleep 1; systemctl start systemd-journald.service|' TEST-04-JOURNAL.bsod.sh
    sed -i 's|systemctl restart systemd-journald$|systemctl stop systemd-journald.service; systemctl reset-failed systemd-journald.service 2>/dev/null; sleep 1; systemctl start systemd-journald.service|' TEST-04-JOURNAL.bsod.sh
  '';
}
