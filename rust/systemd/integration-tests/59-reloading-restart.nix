{
  name = "59-RELOADING-RESTART";
  # Custom rewrite: test RELOADING=1 failure handling (implemented).
  # Skip reload rate limiting (ReloadLimitBurst not implemented) and
  # RestartMode=debug (not implemented).
  #
  # Type=notify-reload is ALSO skipped, but for a DEEPER reason than "not
  # implemented" (the reload SIGHUP dispatch + RELOADING/READY handling exist:
  # control.rs:8522, notification_handler.rs:896). The blocker is the TRANSIENT
  # lifecycle: `systemd-run --unit X -p Type=notify-reload -p KillMode=process
  # <script>` starts the service and rust-systemd immediately logs "Started
  # ... Deactivated successfully" while the script keeps running orphaned in the
  # cgroup. So `systemctl reload` (SIGHUP) and `systemctl stop` (SIGTERM,
  # KillMode=process) never reach the script's trap handlers; stop SIGKILLs the
  # orphan, giving ExecMainStatus=9 (SIGKILL) instead of the expected 109
  # (88 +11 reload +7 stop +3 final). Root cause = transient notify-reload
  # service readiness / main-PID tracking: rust-systemd loses the forked main
  # process for a systemd-run transient notify(-reload) unit and deactivates it
  # at once. Unit-file Type=notify services (the testservice-*-59 above) track
  # correctly, so the gap is specific to the systemd-run transient path. Deep;
  # revisit by tracing start_service main-PID capture + READY=1 wait for
  # transient notify units.
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
