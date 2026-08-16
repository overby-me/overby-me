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

    : "systemd-run --wait --pipe forwards command output"
    # --pipe connects the command's stdout to ours, so we capture its output.
    UNIT="run-pipe-$RANDOM"
    OUT="$(systemd-run --wait --pipe --unit="$UNIT" echo "pipe-test-output")"
    [[ "$OUT" == *"pipe-test-output"* ]]

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
