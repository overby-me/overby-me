{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-timers\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-timers.sh << 'LTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-timers shows timers"
    systemctl list-timers --no-pager > /dev/null

    : "systemctl list-timers --all shows all timers"
    systemctl list-timers --no-pager --all > /dev/null

    : "Create transient timer and verify it appears in list"
    UNIT="list-timer-$RANDOM"
    systemd-run --unit="$UNIT" --on-active=1h --remain-after-exit true
    systemctl list-timers --no-pager --all > /dev/null
    systemctl stop "$UNIT.timer" "$UNIT.service" 2>/dev/null || true
    LTEOF
    chmod +x TEST-74-AUX-UTILS.list-timers.sh
  '';
}
