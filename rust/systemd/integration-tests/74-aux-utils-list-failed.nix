{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-failed\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-failed.sh << 'LFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl --failed returns without error"
    systemctl --failed --no-pager > /dev/null

    : "systemctl --failed --no-legend shows compact output"
    systemctl --failed --no-pager --no-legend > /dev/null || true
    LFEOF
    chmod +x TEST-74-AUX-UTILS.list-failed.sh
  '';
}
