{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.triggered\\-by\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.triggered-by.sh << 'TBEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "TriggeredBy shows the timer that triggers a timed service"
    UNIT="trig-by-$RANDOM"
    systemd-run --unit="$UNIT" --on-active=1h --remain-after-exit true
    TB="$(systemctl show -P TriggeredBy "$UNIT.service" 2>/dev/null || true)"
    # The service pulled in by --on-active must point back at its .timer.
    [[ "$TB" == *"$UNIT.timer"* ]]
    systemctl stop "$UNIT.timer" "$UNIT.service" 2>/dev/null || true
    TBEOF
    chmod +x TEST-74-AUX-UTILS.triggered-by.sh
  '';
}
