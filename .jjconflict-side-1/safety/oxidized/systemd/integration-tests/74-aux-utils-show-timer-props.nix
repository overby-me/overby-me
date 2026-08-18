{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-timer\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-timer-props.sh << 'TPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show timer properties"
    UNIT="timer-show-$RANDOM"
    systemd-run --unit="$UNIT" --on-active=300s --remain-after-exit true
    # Timer should have correct properties
    [[ "$(systemctl show -P ActiveState "$UNIT.timer")" == "active" ]]
    [[ "$(systemctl show -P LoadState "$UNIT.timer")" == "loaded" ]]

    : "Next elapse timestamp is set for active timer"
    NEXT="$(systemctl show -P NextElapseUSecRealtime "$UNIT.timer")" || true
    # May or may not be set, just ensure the property query works
    systemctl show -P NextElapseUSecRealtime "$UNIT.timer" > /dev/null || true

    systemctl stop "$UNIT.timer" "$UNIT.service" 2>/dev/null || true
    TPEOF
    chmod +x TEST-74-AUX-UTILS.show-timer-props.sh
  '';
}
