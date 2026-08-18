{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-timer\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-timer.sh << 'RTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run --on-active creates a timer"
    UNIT="run-timer-$RANDOM"
    systemd-run --unit="$UNIT" --on-active=5min --remain-after-exit true
    systemctl is-active "$UNIT.timer"
    systemctl stop "$UNIT.timer" "$UNIT.service" 2>/dev/null || true

    : "systemd-run --on-boot creates a boot timer"
    UNIT2="run-boot-$RANDOM"
    systemd-run --unit="$UNIT2" --on-boot=1h --remain-after-exit true
    systemctl is-active "$UNIT2.timer"
    systemctl stop "$UNIT2.timer" "$UNIT2.service" 2>/dev/null || true

    : "systemd-run --on-unit-active creates unit-active timer"
    UNIT3="run-unitactive-$RANDOM"
    systemd-run --unit="$UNIT3" --on-unit-active=30s --remain-after-exit true
    systemctl is-active "$UNIT3.timer"
    systemctl stop "$UNIT3.timer" "$UNIT3.service" 2>/dev/null || true
    RTEOF
    chmod +x TEST-74-AUX-UTILS.run-timer.sh
  '';
}
