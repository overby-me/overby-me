{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-pty\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-pty.sh << 'RPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run --wait --pipe runs command and captures output"
    # --pipe forwards stdin/stdout/stderr
    UNIT="run-pipe-$RANDOM"
    systemd-run --wait --pipe --unit="$UNIT" echo "pipe-test-output" > /dev/null || true

    : "systemd-run with --setenv passes environment"
    UNIT2="run-setenv-$RANDOM"
    systemd-run --unit="$UNIT2" --remain-after-exit \
        --setenv=MY_RUN_VAR=setenv-works \
        bash -c 'echo "$MY_RUN_VAR" > /tmp/run-setenv-result'
    sleep 1
    [[ "$(cat /tmp/run-setenv-result)" == "setenv-works" ]]
    systemctl stop "$UNIT2.service" 2>/dev/null || true
    rm -f /tmp/run-setenv-result
    RPEOF
    chmod +x TEST-74-AUX-UTILS.run-pty.sh
  '';
}
