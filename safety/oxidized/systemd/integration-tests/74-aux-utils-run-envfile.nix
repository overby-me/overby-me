{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-envfile\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-envfile.sh << 'REEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    at_exit() {
        set +e
        rm -f /tmp/envfile-test /tmp/envfile-result
    }
    trap at_exit EXIT

    : "systemd-run with -p EnvironmentFile reads env from file"
    cat > /tmp/envfile-test << EOF
    MY_TEST_VAR=hello-from-envfile
    MY_OTHER_VAR=world
    EOF

    UNIT="run-envfile-$RANDOM"
    systemd-run --unit="$UNIT" --remain-after-exit \
        -p EnvironmentFile=/tmp/envfile-test \
        bash -c 'echo "$MY_TEST_VAR $MY_OTHER_VAR" > /tmp/envfile-result'
    sleep 1
    [[ "$(cat /tmp/envfile-result)" == "hello-from-envfile world" ]]
    systemctl stop "$UNIT.service" 2>/dev/null || true
    REEOF
    chmod +x TEST-74-AUX-UTILS.run-envfile.sh
  '';
}
