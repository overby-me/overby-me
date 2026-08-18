{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec-start-pre-post-order\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.exec-start-pre-post-order.sh << 'EOEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/order-test.service
        rm -f /tmp/exec-order-log
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "ExecStartPre runs before ExecStart, ExecStartPost runs after"
    cat > /run/systemd/system/order-test.service << EOF
    [Service]
    Type=oneshot
    ExecStartPre=bash -c 'echo PRE >> /tmp/exec-order-log'
    ExecStart=bash -c 'echo MAIN >> /tmp/exec-order-log'
    ExecStartPost=bash -c 'echo POST >> /tmp/exec-order-log'
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/exec-order-log
    retry systemctl start order-test.service
    [[ "$(sed -n '1p' /tmp/exec-order-log)" == "PRE" ]]
    [[ "$(sed -n '2p' /tmp/exec-order-log)" == "MAIN" ]]
    [[ "$(sed -n '3p' /tmp/exec-order-log)" == "POST" ]]

    # Stop the service from the first test — successful oneshots stay
    # in "active" state, so a second `start` would be a no-op.
    systemctl stop order-test.service || true

    : "ExecStartPre failure prevents ExecStart"
    cat > /run/systemd/system/order-test.service << EOF
    [Service]
    Type=oneshot
    ExecStartPre=false
    ExecStart=bash -c 'echo SHOULD-NOT-RUN >> /tmp/exec-order-log'
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/exec-order-log
    (! systemctl start order-test.service)
    # ExecStart should not have run
    [[ ! -f /tmp/exec-order-log ]] || (! grep -q "SHOULD-NOT-RUN" /tmp/exec-order-log)
    EOEOF
  '';
}
