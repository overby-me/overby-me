{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-all\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-all-props.sh << 'APEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show --all shows all properties"
    PROPS="$(systemctl show --all systemd-journald.service --no-pager | wc -l)"
    [[ "$PROPS" -gt 10 ]]

    : "systemctl show -p with comma-separated props"
    systemctl show -p Id,ActiveState,LoadState systemd-journald.service | grep -q "Id="
    systemctl show -p Id,ActiveState,LoadState systemd-journald.service | grep -q "ActiveState="
    systemctl show -p Id,ActiveState,LoadState systemd-journald.service | grep -q "LoadState="

    : "systemctl show --property=... alternative syntax"
    systemctl show --property=Id systemd-journald.service | grep -q "Id="
    APEOF
    chmod +x TEST-74-AUX-UTILS.show-all-props.sh
  '';
}
