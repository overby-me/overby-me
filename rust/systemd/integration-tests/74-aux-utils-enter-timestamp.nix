{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.enter\\-timestamp\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.enter-timestamp.sh << 'ETEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "ActiveEnterTimestamp is set for active service"
    TS="$(systemctl show -P ActiveEnterTimestamp systemd-journald.service)"
    [[ -n "$TS" ]]

    : "InactiveExitTimestamp is set for active service"
    TS="$(systemctl show -P InactiveExitTimestamp systemd-journald.service)"
    [[ -n "$TS" ]]
    ETEOF
    chmod +x TEST-74-AUX-UTILS.enter-timestamp.sh
  '';
}
