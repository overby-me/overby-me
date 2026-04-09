{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.restart-behavior\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.restart-behavior.sh << 'RESTARTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/restart-test-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "Restart=on-failure restarts on non-zero exit"
    cat > /run/systemd/system/restart-test-onfailure.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'if [ ! -f /tmp/restart-pass ]; then touch /tmp/restart-pass; exit 1; fi'
    RemainAfterExit=yes
    Restart=on-failure
    RestartSec=1
    EOF
    rm -f /tmp/restart-pass
    systemctl daemon-reload
    # First start will fail (exit 1), restart should succeed
    systemctl start restart-test-onfailure.service || true
    # Wait for the auto-restart to succeed
    timeout 15 bash -c 'until systemctl is-active restart-test-onfailure.service 2>/dev/null; do sleep 0.5; done'
    systemctl is-active restart-test-onfailure.service
    [[ "$(systemctl show -P NRestarts restart-test-onfailure.service)" -ge 1 ]]
    systemctl stop restart-test-onfailure.service
    rm -f /tmp/restart-pass

    : "Restart=no does not restart"
    cat > /run/systemd/system/restart-test-no.service << EOF
    [Service]
    Type=oneshot
    ExecStart=false
    Restart=no
    EOF
    systemctl daemon-reload
    systemctl start restart-test-no.service || true
    sleep 2
    [[ "$(systemctl show -P NRestarts restart-test-no.service)" -eq 0 ]]

    RESTARTEOF
  '';
}
