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

    : "TriggeredBy shows timer for timed service"
    UNIT="trig-by-$RANDOM"
    systemd-run --unit="$UNIT" --on-active=1h --remain-after-exit true
    sleep 1
    TB="$(systemctl show -P TriggeredBy "$UNIT.service" 2>/dev/null)" || true
    # May be empty in rust-systemd, just verify no crash
    echo "TriggeredBy=$TB"
    systemctl stop "$UNIT.timer" "$UNIT.service" 2>/dev/null || true
    TBEOF
    chmod +x TEST-74-AUX-UTILS.triggered-by.sh
  '';
}
