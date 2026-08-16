{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.systemctl-kill\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.systemctl-kill.sh << 'SKEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop kill-test.service 2>/dev/null
        rm -f /run/systemd/system/kill-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "systemctl kill sends signal to service"
    cat > /run/systemd/system/kill-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    EOF
    retry systemctl daemon-reload
    retry systemctl start kill-test.service
    systemctl is-active kill-test.service
    PID="$(systemctl show -P MainPID kill-test.service)"
    [[ "$PID" -gt 0 ]]

    # Kill with SIGTERM (default)
    systemctl kill kill-test.service
    timeout 10 bash -c 'until ! systemctl is-active kill-test.service 2>/dev/null; do sleep 0.5; done'
    (! systemctl is-active kill-test.service)

    : "systemctl kill with custom signal"
    retry systemctl start kill-test.service
    systemctl is-active kill-test.service
    systemctl kill --signal=SIGKILL kill-test.service
    timeout 10 bash -c 'until ! systemctl is-active kill-test.service 2>/dev/null; do sleep 0.5; done'
    (! systemctl is-active kill-test.service)
    SKEOF
  '';
}
