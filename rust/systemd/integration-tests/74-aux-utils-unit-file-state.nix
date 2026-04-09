{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.unit\\-file\\-state\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.unit-file-state.sh << 'UFSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "UnitFileState for enabled unit"
    UFS="$(systemctl show -P UnitFileState systemd-journald.service)"
    [[ "$UFS" == "static" || "$UFS" == "enabled" || "$UFS" == "indirect" ]]

    : "UnitFileState for transient unit"
    UNIT="ufs-test-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    UFS="$(systemctl show -P UnitFileState "$UNIT.service")"
    [[ -n "$UFS" ]]
    UFSEOF
    chmod +x TEST-74-AUX-UTILS.unit-file-state.sh
  '';
}
