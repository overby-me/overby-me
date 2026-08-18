{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.control\\-pid\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.control-pid.sh << 'CPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "ControlPID is 0 when no control process"
    UNIT="ctl-pid-$RANDOM"
    systemd-run --unit="$UNIT" sleep 300
    sleep 1
    CPID="$(systemctl show -P ControlPID "$UNIT.service")"
    [[ "$CPID" == "0" ]]
    systemctl stop "$UNIT.service"
    CPEOF
    chmod +x TEST-74-AUX-UTILS.control-pid.sh
  '';
}
