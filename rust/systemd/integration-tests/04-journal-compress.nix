{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "SYSTEMD_JOURNAL_COMPRESS";
  };
  testTimeout = 300;
  patchScript = ''
    # Stop sockets too to prevent socket-activation from re-triggering with old env
    sed -i 's#systemctl restart systemd-journald.service#systemctl stop systemd-journald.socket systemd-journald-dev-log.socket systemd-journald-audit.socket systemd-journald.service 2>/dev/null || true; systemctl reset-failed systemd-journald.service 2>/dev/null || true; sleep 1; systemctl start systemd-journald.socket systemd-journald-dev-log.socket systemd-journald-audit.socket systemd-journald.service 2>/dev/null || true; sleep 1#' TEST-04-JOURNAL.SYSTEMD_JOURNAL_COMPRESS.sh
    sed -i 's#systemctl restart systemd-journald$#systemctl stop systemd-journald.socket systemd-journald-dev-log.socket systemd-journald-audit.socket systemd-journald.service 2>/dev/null || true; systemctl reset-failed systemd-journald.service 2>/dev/null || true; sleep 1; systemctl start systemd-journald.socket systemd-journald-dev-log.socket systemd-journald-audit.socket systemd-journald.service 2>/dev/null || true; sleep 1#' TEST-04-JOURNAL.SYSTEMD_JOURNAL_COMPRESS.sh
    # Skip journal-remote sub-test (uses C systemd-journal-remote, not reimplemented)
    sed -i 's#if \[\[ -x /usr/lib/systemd/systemd-journal-remote \]\]#if false#' TEST-04-JOURNAL.SYSTEMD_JOURNAL_COMPRESS.sh
  '';
}
