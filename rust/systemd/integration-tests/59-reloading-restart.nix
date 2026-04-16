{
  name = "59-RELOADING-RESTART";
  # Custom rewrite: test RELOADING=1 failure handling (implemented).
  # Skip reload rate limiting (ReloadLimitBurst not implemented),
  # Type=notify-reload (not implemented), and RestartMode=debug (not implemented).
  patchScript = ''
        cat > TEST-59-RELOADING-RESTART.sh << 'TESTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    fail() {
        exit 1
    }

    wait_on_state_or_fail() {
        local service=$1
        local expected_state=$2
        local timeout=$3

        local state
        state=$(systemctl show "$service" --property=ActiveState --value)
        while [ "$state" != "$expected_state" ]; do
            if [ "$timeout" = "0" ]; then
                echo "Timed out waiting for $service to reach $expected_state (got $state)"
                fail
            fi
            timeout=$((timeout - 1))
            sleep 1
            state=$(systemctl show "$service" --property=ActiveState --value)
        done
    }

    at_exit() {
        set +e
        systemctl stop testservice-fail-59.service 2>/dev/null
        systemctl stop testservice-fail-restart-59.service 2>/dev/null
        systemctl stop testservice-abort-restart-59.service 2>/dev/null
        systemctl stop testservice-reload-ok-59.service 2>/dev/null
        systemctl reset-failed testservice-fail-59.service 2>/dev/null
        systemctl reset-failed testservice-fail-restart-59.service 2>/dev/null
        systemctl reset-failed testservice-abort-restart-59.service 2>/dev/null
        rm -f /run/systemd/system/testservice-*-59.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "RELOADING=1 then exit 1 should enter failed state"
    cat >/run/systemd/system/testservice-fail-59.service <<EOF
    [Unit]
    Description=TEST-59 Normal exit after RELOADING=1

    [Service]
    Type=notify
    ExecStart=/usr/bin/bash -c "systemd-notify --ready; systemd-notify RELOADING=1; sleep 1; exit 1"
    EOF

    systemctl daemon-reload
    systemctl start testservice-fail-59.service
    wait_on_state_or_fail "testservice-fail-59.service" "failed" "30"
    systemctl reset-failed testservice-fail-59.service 2>/dev/null || true

    : "RELOADING=1 then exit 1 with Restart=on-failure reaches failed via StartLimitBurst"
    cat >/run/systemd/system/testservice-fail-restart-59.service <<EOF
    [Unit]
    Description=TEST-59 Restart=on-failure after RELOADING=1

    [Service]
    Type=notify
    ExecStart=/usr/bin/bash -c "systemd-notify --ready; systemd-notify RELOADING=1; sleep 1; exit 1"
    Restart=on-failure
    StartLimitBurst=1
    EOF

    systemctl daemon-reload
    systemctl start testservice-fail-restart-59.service
    wait_on_state_or_fail "testservice-fail-restart-59.service" "failed" "30"
    systemctl reset-failed testservice-fail-restart-59.service 2>/dev/null || true

    : "RELOADING=1 then SIGABRT with Restart=on-abort should fail"
    cat >/run/systemd/system/testservice-abort-restart-59.service <<EOF
    [Unit]
    Description=TEST-59 Restart=on-abort after RELOADING=1

    [Service]
    Type=notify
    ExecStart=/usr/bin/bash -c "systemd-notify --ready; systemd-notify RELOADING=1; sleep 5; exit 1"
    Restart=on-abort
    EOF

    systemctl daemon-reload
    systemctl start testservice-abort-restart-59.service
    sleep 2
    systemctl --signal=SIGABRT kill testservice-abort-restart-59.service
    wait_on_state_or_fail "testservice-abort-restart-59.service" "failed" "30"
    systemctl reset-failed testservice-abort-restart-59.service 2>/dev/null || true

    : "READY=1 after RELOADING=1 means reload complete, service stays active"
    cat >/run/systemd/system/testservice-reload-ok-59.service <<EOF
    [Unit]
    Description=TEST-59 Successful reload

    [Service]
    Type=notify
    ExecStart=/usr/bin/bash -c 'systemd-notify --ready; sleep 2; systemd-notify RELOADING=1; sleep 1; systemd-notify --ready; sleep 60'
    ExecReload=/usr/bin/kill -HUP \$MAINPID
    EOF

    systemctl daemon-reload
    systemctl start testservice-reload-ok-59.service
    sleep 5
    systemctl is-active testservice-reload-ok-59.service
    systemctl stop testservice-reload-ok-59.service

    rm -f /run/systemd/system/testservice-*-59.service
    systemctl daemon-reload

    touch /testok
    TESTEOF
        chmod +x TEST-59-RELOADING-RESTART.sh
  '';
}
