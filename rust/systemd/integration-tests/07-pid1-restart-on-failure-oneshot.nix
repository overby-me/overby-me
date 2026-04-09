{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.restart-on-failure-oneshot\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.restart-on-failure-oneshot.sh << 'ROFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop restart-oneshot-test.service 2>/dev/null
        rm -f /run/systemd/system/restart-oneshot-test.service
        rm -f /tmp/restart-oneshot-count
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "Restart=on-failure restarts oneshot on failure"
    # This service fails on first two runs, succeeds on third
    cat > /run/systemd/system/restart-oneshot-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    Restart=on-failure
    RestartSec=1
    ExecStart=bash -c 'COUNT=0; [[ -f /tmp/restart-oneshot-count ]] && COUNT=\$(cat /tmp/restart-oneshot-count); echo \$((COUNT + 1)) > /tmp/restart-oneshot-count; [[ \$COUNT -ge 2 ]]'
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/restart-oneshot-count
    systemctl start restart-oneshot-test.service || true
    # Wait for the service to eventually succeed after retries
    timeout 30 bash -c 'until systemctl is-active restart-oneshot-test.service 2>/dev/null; do sleep 1; done'
    systemctl is-active restart-oneshot-test.service
    # Should have run at least 3 times
    [[ "$(cat /tmp/restart-oneshot-count)" -ge 3 ]]
    ROFEOF
  '';
}
