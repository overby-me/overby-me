{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cat\\-content\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.cat-content.sh << 'CCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    at_exit() {
        set +e
        rm -f /run/systemd/system/cat-test-unit.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "systemctl cat shows unit file content"
    cat > /run/systemd/system/cat-test-unit.service << EOF
    [Unit]
    Description=Cat content test
    [Service]
    Type=oneshot
    ExecStart=true
    EOF
    systemctl daemon-reload
    systemctl cat cat-test-unit.service | grep -q "Description=Cat content test"
    systemctl cat cat-test-unit.service | grep -q "ExecStart=true"

    : "systemctl cat with drop-in shows override"
    mkdir -p /run/systemd/system/cat-test-unit.service.d
    cat > /run/systemd/system/cat-test-unit.service.d/override.conf << EOF
    [Service]
    Environment=FOO=bar
    EOF
    systemctl daemon-reload
    systemctl cat cat-test-unit.service | grep -q "Environment=FOO=bar"
    rm -rf /run/systemd/system/cat-test-unit.service.d
    CCEOF
    chmod +x TEST-74-AUX-UTILS.cat-content.sh
  '';
}
