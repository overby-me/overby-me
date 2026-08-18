{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.ordering\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.ordering.sh << 'ORDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop order-test-{a,b,c}.service 2>/dev/null
        rm -f /run/systemd/system/order-test-{a,b,c}.service
        rm -f /tmp/order-test-*
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "After= ensures ordering"
    cat > /run/systemd/system/order-test-a.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'sleep 0.5; echo a > /tmp/order-test-a'
    EOF
    cat > /run/systemd/system/order-test-b.service << EOF
    [Unit]
    After=order-test-a.service
    Wants=order-test-a.service
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'test -f /tmp/order-test-a && echo b > /tmp/order-test-b'
    EOF
    rm -f /tmp/order-test-a /tmp/order-test-b
    systemctl daemon-reload
    systemctl start order-test-b.service
    systemctl is-active order-test-a.service
    systemctl is-active order-test-b.service
    [[ -f /tmp/order-test-a ]]
    [[ -f /tmp/order-test-b ]]
    systemctl stop order-test-a.service order-test-b.service
    rm -f /tmp/order-test-a /tmp/order-test-b

    : "Before= ensures reverse ordering"
    cat > /run/systemd/system/order-test-c.service << EOF
    [Unit]
    Before=order-test-a.service
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'echo c > /tmp/order-test-c'
    EOF
    rm -f /tmp/order-test-a /tmp/order-test-c
    # Rewrite a to check c exists
    cat > /run/systemd/system/order-test-a.service << EOF
    [Unit]
    Wants=order-test-c.service
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'test -f /tmp/order-test-c && echo a2 > /tmp/order-test-a'
    EOF
    systemctl daemon-reload
    systemctl start order-test-a.service
    [[ -f /tmp/order-test-c ]]
    [[ -f /tmp/order-test-a ]]
    ORDEOF
  '';
}
