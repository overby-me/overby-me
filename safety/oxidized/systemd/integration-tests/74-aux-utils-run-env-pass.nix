{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-env\\-pass\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-env-pass.sh << 'REPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run passes environment with -p"
    UNIT="env-pass-$RANDOM"
    systemd-run --wait --unit="$UNIT" \
        -p Environment="TEST_PASS_VAR=hello-env" \
        bash -c 'echo "$TEST_PASS_VAR" > /tmp/env-pass-result'
    [[ "$(cat /tmp/env-pass-result)" == "hello-env" ]]
    rm -f /tmp/env-pass-result

    : "systemd-run --setenv passes environment"
    UNIT="setenv-$RANDOM"
    TEST_SETENV_VAR=from-setenv systemd-run --wait --unit="$UNIT" \
        --setenv=TEST_SETENV_VAR \
        bash -c 'echo "$TEST_SETENV_VAR" > /tmp/setenv-result'
    [[ "$(cat /tmp/setenv-result)" == "from-setenv" ]]
    rm -f /tmp/setenv-result
    REPEOF
    chmod +x TEST-74-AUX-UTILS.run-env-pass.sh
  '';
}
