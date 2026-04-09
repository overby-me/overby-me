{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cat\\-dropin\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.cat-dropin.sh << 'CDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -rf /run/systemd/system/cat-dropin-test.service /run/systemd/system/cat-dropin-test.service.d
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "systemctl cat shows unit file and drop-ins"
    cat > /run/systemd/system/cat-dropin-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=true
    EOF
    mkdir -p /run/systemd/system/cat-dropin-test.service.d
    cat > /run/systemd/system/cat-dropin-test.service.d/override.conf << EOF
    [Service]
    Environment=FOO=bar
    EOF
    systemctl daemon-reload

    OUTPUT=$(systemctl cat cat-dropin-test.service)
    echo "$OUTPUT" | grep -q "ExecStart=true"
    echo "$OUTPUT" | grep -q "FOO=bar"
    CDEOF
    chmod +x TEST-74-AUX-UTILS.cat-dropin.sh
  '';
}
