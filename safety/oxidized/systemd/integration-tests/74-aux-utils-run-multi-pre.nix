{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-multi\\-pre\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-multi-pre.sh << 'RMPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run with -p ExecStartPre runs pre-command"
    UNIT="run-pre-$RANDOM"
    systemd-run --wait --unit="$UNIT" \
        -p ExecStartPre="touch /tmp/$UNIT-pre" \
        true
    [[ -f "/tmp/$UNIT-pre" ]]
    rm -f "/tmp/$UNIT-pre"
    RMPEOF
    chmod +x TEST-74-AUX-UTILS.run-multi-pre.sh
  '';
}
