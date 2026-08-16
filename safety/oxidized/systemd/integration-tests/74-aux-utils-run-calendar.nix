{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-calendar\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-calendar.sh << 'CALEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run --on-calendar creates a calendar timer"
    UNIT="run-cal-$RANDOM"
    systemd-run --unit="$UNIT" --on-calendar="*:*:0/10" --remain-after-exit true
    systemctl is-active "$UNIT.timer"
    grep -q "OnCalendar=" "/run/systemd/transient/$UNIT.timer"
    systemctl stop "$UNIT.timer" "$UNIT.service" 2>/dev/null || true

    : "systemd-run --on-startup creates startup timer"
    UNIT2="run-startup-$RANDOM"
    systemd-run --unit="$UNIT2" --on-startup=1h --remain-after-exit true
    systemctl is-active "$UNIT2.timer"
    systemctl stop "$UNIT2.timer" "$UNIT2.service" 2>/dev/null || true
    CALEOF
    chmod +x TEST-74-AUX-UTILS.run-calendar.sh
  '';
}
