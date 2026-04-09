{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.reload\\-restart\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.reload-restart.sh << 'RREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    at_exit() {
        set +e
        systemctl stop reload-restart-test.service 2>/dev/null
        rm -f /run/systemd/system/reload-restart-test.service
        rm -f /tmp/reload-restart-*
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "systemctl reload-or-restart works for running service"
    cat > /run/systemd/system/reload-restart-test.service << EOF
    [Unit]
    Description=Reload restart test
    [Service]
    Type=simple
    ExecStart=sleep infinity
    ExecReload=touch /tmp/reload-restart-reloaded
    EOF
    systemctl daemon-reload
    systemctl start reload-restart-test.service
    [[ "$(systemctl show -P ActiveState reload-restart-test.service)" == "active" ]]

    systemctl reload-or-restart reload-restart-test.service
    # Service should still be active after reload-or-restart
    sleep 1
    [[ "$(systemctl show -P ActiveState reload-restart-test.service)" == "active" ]]

    : "systemctl try-restart only restarts if running"
    systemctl try-restart reload-restart-test.service
    sleep 1
    [[ "$(systemctl show -P ActiveState reload-restart-test.service)" == "active" ]]
    RREOF
    chmod +x TEST-74-AUX-UTILS.reload-restart.sh
  '';
}
