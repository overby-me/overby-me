{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-result\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-result.sh << 'SREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "Result=success for successfully completed service"
    UNIT="result-test-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    RESULT="$(systemctl show -P Result "$UNIT.service")"
    [[ "$RESULT" == "success" ]]

    : "Result for failed service"
    UNIT2="result-fail-$RANDOM"
    (! systemd-run --wait --unit="$UNIT2" false)
    RESULT="$(systemctl show -P Result "$UNIT2.service")"
    [[ -n "$RESULT" ]]
    SREOF
    chmod +x TEST-74-AUX-UTILS.show-result.sh
  '';
}
