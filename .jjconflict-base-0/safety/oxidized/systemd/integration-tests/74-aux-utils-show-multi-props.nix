{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-multi\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-multi-props.sh << 'SMPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show -p with multiple --property flags"
    OUT="$(systemctl show systemd-journald.service -P ActiveState -P SubState)"
    [[ -n "$OUT" ]]

    : "systemctl show --property with comma-separated properties"
    OUT="$(systemctl show systemd-journald.service --property=ActiveState,SubState)"
    echo "$OUT" | grep -q "ActiveState="
    echo "$OUT" | grep -q "SubState="

    : "systemctl show for Type property"
    TYPE="$(systemctl show -P Type systemd-journald.service)"
    [[ -n "$TYPE" ]]
    SMPEOF
    chmod +x TEST-74-AUX-UTILS.show-multi-props.sh
  '';
}
