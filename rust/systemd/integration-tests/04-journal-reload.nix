{
  name = "04-JOURNAL";
  testEnv = {
    TEST_MATCH_SUBTEST = "reload";
  };
  testTimeout = 600;
  patchScript = ''
    # Use stop+reset-failed+start instead of restart
    sed -i 's|systemctl restart systemd-journald.service|systemctl stop systemd-journald.service; systemctl reset-failed systemd-journald.service 2>/dev/null; sleep 1; systemctl start systemd-journald.service|' TEST-04-JOURNAL.reload.sh
    sed -i 's|systemctl restart systemd-journald$|systemctl stop systemd-journald.service; systemctl reset-failed systemd-journald.service 2>/dev/null; sleep 1; systemctl start systemd-journald.service|' TEST-04-JOURNAL.reload.sh
  '';
}
