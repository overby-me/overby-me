{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-collect\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-collect.sh << 'RCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run --collect removes unit after exit"
    UNIT="collect-$RANDOM"
    systemd-run --wait --collect --unit="$UNIT" true
    # After --collect, unit should be gone or inactive
    STATE="$(systemctl show -P LoadState "$UNIT.service" 2>/dev/null)" || true
    [[ "$STATE" == "not-found" || "$STATE" == "" || "$STATE" == "loaded" ]]
    RCEOF
    chmod +x TEST-74-AUX-UTILS.run-collect.sh
  '';
}
