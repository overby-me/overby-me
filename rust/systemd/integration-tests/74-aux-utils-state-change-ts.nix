{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.state\\-change\\-ts\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.state-change-ts.sh << 'SCTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "StateChangeTimestamp is set for active service"
    TS="$(systemctl show -P StateChangeTimestamp systemd-journald.service)"
    [[ -n "$TS" ]]
    SCTEOF
    chmod +x TEST-74-AUX-UTILS.state-change-ts.sh
  '';
}
