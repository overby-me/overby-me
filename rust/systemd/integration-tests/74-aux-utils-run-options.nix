{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-options\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-options.sh << 'ROEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run with --uid runs as specified user"
    UNIT="run-uid-$RANDOM"
    systemd-run --wait --unit="$UNIT" --uid=nobody id > /dev/null || true

    : "systemd-run with --nice sets nice level"
    UNIT2="run-nice-$RANDOM"
    systemd-run --unit="$UNIT2" --remain-after-exit \
        --nice=5 \
        bash -c 'nice > /tmp/run-nice-result'
    sleep 1
    [[ "$(cat /tmp/run-nice-result)" == "5" ]]
    systemctl stop "$UNIT2.service" 2>/dev/null || true
    rm -f /tmp/run-nice-result
    ROEOF
    chmod +x TEST-74-AUX-UTILS.run-options.sh
  '';
}
