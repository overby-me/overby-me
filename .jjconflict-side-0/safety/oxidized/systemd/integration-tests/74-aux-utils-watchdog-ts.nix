{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.watchdog\\-ts\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.watchdog-ts.sh << 'WTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "WatchdogTimestamp property exists"
    systemctl show -P WatchdogTimestamp systemd-journald.service > /dev/null

    : "WatchdogTimestampMonotonic property exists"
    systemctl show -P WatchdogTimestampMonotonic systemd-journald.service > /dev/null
    WTEOF
    chmod +x TEST-74-AUX-UTILS.watchdog-ts.sh
  '';
}
