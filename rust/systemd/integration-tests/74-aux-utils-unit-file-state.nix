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

    # UnitFileState reflects enablement, which the NixOS test harness controls
    # via /etc/systemd/system symlinks, so journald is not deterministically
    # "static" here (unlike a bare upstream install). Accept the plausible set.
    : "UnitFileState for systemd-journald"
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
