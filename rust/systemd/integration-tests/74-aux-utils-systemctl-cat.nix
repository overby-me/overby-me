{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.systemctl\\-cat\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.systemctl-cat.sh << 'SCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/cat-test.service
        rm -rf /run/systemd/system/cat-test.service.d
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "systemctl cat shows unit file contents"
    cat > /run/systemd/system/cat-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=echo hello-cat
    EOF
    systemctl daemon-reload
    systemctl cat cat-test.service | grep -q "ExecStart=echo hello-cat"

    : "systemctl cat shows drop-in contents"
    mkdir -p /run/systemd/system/cat-test.service.d
    cat > /run/systemd/system/cat-test.service.d/override.conf << EOF
    [Service]
    Environment=CAT_VAR=test
    EOF
    systemctl daemon-reload
    OUTPUT=$(systemctl cat cat-test.service)
    echo "$OUTPUT" | grep -q "ExecStart=echo hello-cat"
    echo "$OUTPUT" | grep -q "CAT_VAR=test"

    : "systemctl cat for nonexistent unit fails"
    (! systemctl cat nonexistent-unit-12345.service)
    SCEOF
    chmod +x TEST-74-AUX-UTILS.systemctl-cat.sh
  '';
}
