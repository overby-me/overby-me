{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-slice\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-slice.sh << 'RSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run with --slice places service in specified slice"
    UNIT="run-slice-$RANDOM"
    systemd-run --unit="$UNIT" --slice=system --remain-after-exit true
    sleep 1
    SLICE="$(systemctl show -P Slice "$UNIT.service")"
    [[ "$SLICE" == "system.slice" || "$SLICE" == "system" ]]
    systemctl stop "$UNIT.service" 2>/dev/null || true
    RSEOF
    chmod +x TEST-74-AUX-UTILS.run-slice.sh
  '';
}
