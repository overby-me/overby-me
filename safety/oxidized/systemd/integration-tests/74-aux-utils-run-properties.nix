{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-properties\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-properties.sh << 'RPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemd-run with --description"
    UNIT="run-prop-$RANDOM"
    systemd-run --unit="$UNIT" --description="Test property service" \
        --remain-after-exit true
    sleep 1
    DESC="$(systemctl show -P Description "$UNIT.service")"
    [[ "$DESC" == "Test property service" ]]
    systemctl stop "$UNIT.service" 2>/dev/null || true

    : "systemd-run with environment variables"
    UNIT3="run-prop3-$RANDOM"
    systemd-run --wait --unit="$UNIT3" \
        -p Environment="TESTVAR=hello" \
        bash -c '[[ "$TESTVAR" == "hello" ]]'

    : "systemd-run with WorkingDirectory"
    UNIT4="run-prop4-$RANDOM"
    systemd-run --wait --unit="$UNIT4" \
        -p WorkingDirectory=/tmp \
        bash -c '[[ "$(pwd)" == "/tmp" ]]'
    RPEOF
    chmod +x TEST-74-AUX-UTILS.run-properties.sh
  '';
}
