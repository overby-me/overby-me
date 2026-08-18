{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.mask\\-unmask\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.mask-unmask.sh << 'MMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    at_exit() {
        set +e
        systemctl unmask mask-test-unit.service 2>/dev/null
        rm -f /run/systemd/system/mask-test-unit.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "Create a test service"
    cat > /run/systemd/system/mask-test-unit.service << EOF
    [Unit]
    Description=Mask test unit
    [Service]
    Type=oneshot
    ExecStart=true
    EOF
    systemctl daemon-reload

    : "systemctl mask creates a symlink to /dev/null"
    systemctl mask mask-test-unit.service
    [[ -L /etc/systemd/system/mask-test-unit.service ]] || \
        [[ -L /run/systemd/system/mask-test-unit.service ]]

    : "systemctl unmask removes the mask"
    systemctl unmask mask-test-unit.service
    systemctl daemon-reload
    # Service should be startable again after unmask
    systemctl start mask-test-unit.service
    MMEOF
    chmod +x TEST-74-AUX-UTILS.mask-unmask.sh
  '';
}
