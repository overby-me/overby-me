{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-pid\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-pid-props.sh << 'PPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show MainPID for running service"
    UNIT="pid-test-$RANDOM"
    systemd-run --unit="$UNIT" sleep 300
    sleep 1
    PID="$(systemctl show -P MainPID "$UNIT.service")"
    [[ "$PID" -gt 0 ]]
    kill -0 "$PID"
    systemctl stop "$UNIT.service"

    : "systemctl show ExecMainPID for completed service"
    UNIT2="pid-done-$RANDOM"
    systemd-run --wait --unit="$UNIT2" true
    # After completion, MainPID should be 0
    PID="$(systemctl show -P MainPID "$UNIT2.service")"
    [[ "$PID" -eq 0 ]]
    PPEOF
    chmod +x TEST-74-AUX-UTILS.show-pid-props.sh
  '';
}
