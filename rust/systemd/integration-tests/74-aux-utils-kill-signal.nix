{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.kill\\-signal\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.kill-signal.sh << 'KSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl kill sends signal to service"
    UNIT="kill-test-$RANDOM"
    systemd-run --unit="$UNIT" sleep 300
    sleep 1
    systemctl is-active "$UNIT.service"
    systemctl kill "$UNIT.service"
    sleep 1
    (! systemctl is-active "$UNIT.service")
    systemctl reset-failed "$UNIT.service" 2>/dev/null || true
    KSEOF
    chmod +x TEST-74-AUX-UTILS.kill-signal.sh
  '';
}
