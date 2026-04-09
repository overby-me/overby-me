{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec\\-timestamps\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.exec-timestamps.sh << 'XTSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "ExecMainStartTimestamp is set after service runs"
    UNIT="exec-ts-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    TS="$(systemctl show -P ExecMainStartTimestamp "$UNIT.service")"
    [[ -n "$TS" ]]

    : "ExecMainExitTimestamp is set after service completes"
    TS="$(systemctl show -P ExecMainExitTimestamp "$UNIT.service")"
    [[ -n "$TS" ]]
    XTSEOF
    chmod +x TEST-74-AUX-UTILS.exec-timestamps.sh
  '';
}
