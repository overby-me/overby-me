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

    : "systemctl --failed --no-legend omits the column header"
    systemctl --failed --no-pager --no-legend > /tmp/failed-nl.txt
    # --no-legend must suppress the "UNIT LOAD ACTIVE SUB ..." header line.
    ! grep -qiE "^\s*UNIT\s+LOAD" /tmp/failed-nl.txt
    rm -f /tmp/failed-nl.txt
    LFEOF
    chmod +x TEST-74-AUX-UTILS.list-failed.sh
  '';
}
