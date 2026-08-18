{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.timer\\-show\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.timer-show-props.sh << 'TPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show for transient timer"
    UNIT="timer-show-$RANDOM"
    systemd-run --on-active=999h --unit="$UNIT" true
    systemctl show "$UNIT.timer" -P ActiveState | grep -q "active"
    systemctl show "$UNIT.timer" -P Id | grep -q "$UNIT.timer"
    systemctl stop "$UNIT.timer"
    TPEOF
    chmod +x TEST-74-AUX-UTILS.timer-show-props.sh
  '';
}
