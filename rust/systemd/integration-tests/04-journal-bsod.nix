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
    # Stop sockets too to prevent socket-activation from re-triggering with old env
    sed -i 's#systemctl restart systemd-journald.service#systemctl stop systemd-journald.socket systemd-journald-dev-log.socket systemd-journald-audit.socket systemd-journald.service 2>/dev/null || true; systemctl reset-failed systemd-journald.service 2>/dev/null || true; sleep 1; systemctl start systemd-journald.socket systemd-journald-dev-log.socket systemd-journald-audit.socket systemd-journald.service 2>/dev/null || true; sleep 1#' TEST-04-JOURNAL.bsod.sh
    sed -i 's#systemctl restart systemd-journald$#systemctl stop systemd-journald.socket systemd-journald-dev-log.socket systemd-journald-audit.socket systemd-journald.service 2>/dev/null || true; systemctl reset-failed systemd-journald.service 2>/dev/null || true; sleep 1; systemctl start systemd-journald.socket systemd-journald-dev-log.socket systemd-journald-audit.socket systemd-journald.service 2>/dev/null || true; sleep 1#' TEST-04-JOURNAL.bsod.sh
  '';
}
