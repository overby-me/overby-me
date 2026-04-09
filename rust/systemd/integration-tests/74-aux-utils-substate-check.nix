{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.substate\\-check\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.substate-check.sh << 'SBEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "SubState=running for active long-running service"
    UNIT="sub-run-$RANDOM"
    systemd-run --unit="$UNIT" sleep 300
    sleep 1
    SS="$(systemctl show -P SubState "$UNIT.service")"
    [[ "$SS" == "running" ]]
    systemctl stop "$UNIT.service"

    : "SubState=dead for stopped service"
    SS="$(systemctl show -P SubState "$UNIT.service")"
    [[ "$SS" == "dead" || "$SS" == "failed" ]]
    systemctl reset-failed "$UNIT.service" 2>/dev/null || true
    SBEOF
    chmod +x TEST-74-AUX-UTILS.substate-check.sh
  '';
}
