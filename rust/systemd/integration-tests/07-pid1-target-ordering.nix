{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.target-ordering\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.target-ordering.sh << 'TOEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop order-test-target.target order-test-a.service order-test-b.service 2>/dev/null
        rm -f /run/systemd/system/order-test-*.{target,service}
        rm -f /tmp/order-test-log
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "Wants= + After= ordering: B starts before A"
    cat > /run/systemd/system/order-test-b.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'echo B >> /tmp/order-test-log'
    EOF

    cat > /run/systemd/system/order-test-a.service << EOF
    [Unit]
    Wants=order-test-b.service
    After=order-test-b.service
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'echo A >> /tmp/order-test-log'
    EOF

    retry systemctl daemon-reload
    rm -f /tmp/order-test-log
    retry systemctl start order-test-a.service
    # B should have started before A
    [[ "$(sed -n '1p' /tmp/order-test-log)" == "B" ]]
    [[ "$(sed -n '2p' /tmp/order-test-log)" == "A" ]]
    TOEOF
  '';
}
