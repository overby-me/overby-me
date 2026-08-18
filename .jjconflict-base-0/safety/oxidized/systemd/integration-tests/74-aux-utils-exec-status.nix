{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec\\-status\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.exec-status.sh << 'ESEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "ExecMainStatus=0 for successful service"
    UNIT="exec-ok-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    STATUS="$(systemctl show -P ExecMainStatus "$UNIT.service")"
    [[ "$STATUS" == "0" ]]

    : "ExecMainStatus non-zero for failed service"
    UNIT2="exec-fail-$RANDOM"
    systemd-run --wait --unit="$UNIT2" bash -c 'exit 42' || true
    STATUS="$(systemctl show -P ExecMainStatus "$UNIT2.service")"
    [[ "$STATUS" == "42" ]]
    systemctl reset-failed "$UNIT2.service" 2>/dev/null || true
    ESEOF
    chmod +x TEST-74-AUX-UTILS.exec-status.sh
  '';
}
