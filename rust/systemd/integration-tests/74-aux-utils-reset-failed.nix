{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.reset\\-failed\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.reset-failed.sh << 'RFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    at_exit() {
        set +e
        systemctl stop rf-test.service 2>/dev/null
        systemctl reset-failed rf-test.service 2>/dev/null
        rm -f /run/systemd/system/rf-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "Failed service shows failed state"
    cat > /run/systemd/system/rf-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=false
    EOF
    systemctl daemon-reload

    systemctl start rf-test.service || true
    sleep 1
    systemctl is-failed rf-test.service

    : "reset-failed clears failed state"
    systemctl reset-failed rf-test.service
    (! systemctl is-failed rf-test.service)
    RFEOF
    chmod +x TEST-74-AUX-UTILS.reset-failed.sh
  '';
}
