{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.watchdog\\-usec\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.watchdog-usec.sh << 'WUEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "WatchdogUSec defaults to 0"
    UNIT="wdog-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    WD="$(systemctl show -P WatchdogUSec "$UNIT.service")"
    [[ "$WD" == "0" || "$WD" == "infinity" || "$WD" == "" ]]
    WUEOF
    chmod +x TEST-74-AUX-UTILS.watchdog-usec.sh
  '';
}
