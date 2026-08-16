{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.after\\-timestamp\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.after-timestamp.sh << 'ATEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "InactiveEnterTimestamp set after service stops"
    UNIT="ats-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    TS="$(systemctl show -P InactiveEnterTimestamp "$UNIT.service")"
    [[ -n "$TS" ]]

    : "ActiveEnterTimestamp was set during run"
    TS2="$(systemctl show -P ActiveEnterTimestamp "$UNIT.service")"
    [[ -n "$TS2" ]]
    ATEOF
    chmod +x TEST-74-AUX-UTILS.after-timestamp.sh
  '';
}
