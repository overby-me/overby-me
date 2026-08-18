{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec\\-main\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.exec-main-props.sh << 'EMPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "MainPID is set for running service"
    UNIT="emp-$RANDOM"
    systemd-run --unit="$UNIT" sleep 300
    sleep 1
    PID="$(systemctl show -P MainPID "$UNIT.service")"
    [[ -n "$PID" && "$PID" != "0" ]]
    systemctl stop "$UNIT.service"

    : "ExecMainStartTimestamp is set after service runs"
    UNIT2="emp2-$RANDOM"
    systemd-run --wait --unit="$UNIT2" true
    TS="$(systemctl show -P ExecMainStartTimestamp "$UNIT2.service")"
    [[ -n "$TS" ]]
    EMPEOF
    chmod +x TEST-74-AUX-UTILS.exec-main-props.sh
  '';
}
