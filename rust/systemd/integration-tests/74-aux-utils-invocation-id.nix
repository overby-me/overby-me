{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.invocation\\-id\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.invocation-id.sh << 'IIEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show InvocationID is non-empty for active service"
    INV="$(systemctl show -P InvocationID systemd-journald.service)"
    [[ -n "$INV" ]]

    : "InvocationID changes on restart"
    UNIT="inv-test-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    INV1="$(systemctl show -P InvocationID "$UNIT.service")"
    systemd-run --wait --unit="$UNIT" true 2>/dev/null || true
    IIEOF
    chmod +x TEST-74-AUX-UTILS.invocation-id.sh
  '';
}
