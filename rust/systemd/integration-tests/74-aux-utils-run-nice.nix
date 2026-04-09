{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-nice\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-nice.sh << 'RNEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run with --nice sets priority"
    UNIT="run-nice-$RANDOM"
    systemd-run --wait --unit="$UNIT" -p Nice=5 \
        bash -c 'nice > /tmp/nice-result'
    [[ "$(cat /tmp/nice-result)" == "5" ]]
    rm -f /tmp/nice-result
    RNEOF
    chmod +x TEST-74-AUX-UTILS.run-nice.sh
  '';
}
