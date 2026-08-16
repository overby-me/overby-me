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
    systemd-run --unit="$UNIT" sleep 3600
    INV1="$(systemctl show -P InvocationID "$UNIT.service")"
    [[ -n "$INV1" ]]
    systemctl restart "$UNIT.service"
    INV2="$(systemctl show -P InvocationID "$UNIT.service")"
    [[ -n "$INV2" ]]
    # A fresh invocation must get a fresh 128-bit InvocationID.
    [[ "$INV1" != "$INV2" ]]
    systemctl stop "$UNIT.service" 2>/dev/null || true
    IIEOF
    chmod +x TEST-74-AUX-UTILS.invocation-id.sh
  '';
}
