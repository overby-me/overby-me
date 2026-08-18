{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-remain\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-remain-props.sh << 'RRPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run --remain-after-exit keeps service active"
    UNIT="remain-prop-$RANDOM"
    systemd-run --unit="$UNIT" --remain-after-exit \
        -p Environment=TEST_REMAIN=yes \
        true
    sleep 1
    systemctl is-active "$UNIT.service"
    systemctl stop "$UNIT.service"
    RRPEOF
    chmod +x TEST-74-AUX-UTILS.run-remain-props.sh
  '';
}
