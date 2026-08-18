{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-workdir\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-workdir.sh << 'RWEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run with WorkingDirectory"
    UNIT="run-wd-$RANDOM"
    systemd-run --wait --unit="$UNIT" \
        -p WorkingDirectory=/tmp \
        bash -c 'pwd > /tmp/workdir-result'
    [[ "$(cat /tmp/workdir-result)" == "/tmp" ]]
    rm -f /tmp/workdir-result
    RWEOF
    chmod +x TEST-74-AUX-UTILS.run-workdir.sh
  '';
}
