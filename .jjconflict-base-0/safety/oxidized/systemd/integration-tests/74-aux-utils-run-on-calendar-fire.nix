{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-on\\-calendar\\-fire\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-on-calendar-fire.sh << 'ROCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run --on-calendar creates and starts timer"
    UNIT="on-cal-fire-$RANDOM"
    systemd-run --unit="$UNIT" \
        --on-calendar="*:*:0/15" \
        --remain-after-exit true
    systemctl is-active "$UNIT.timer"
    [[ "$(systemctl show -P LoadState "$UNIT.timer")" == "loaded" ]]
    systemctl stop "$UNIT.timer" "$UNIT.service" 2>/dev/null || true
    ROCEOF
    chmod +x TEST-74-AUX-UTILS.run-on-calendar-fire.sh
  '';
}
