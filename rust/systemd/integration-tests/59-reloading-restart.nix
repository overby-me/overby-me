{
  name = "59-RELOADING-RESTART";
  # Custom rewrite: test RELOADING=1 failure handling (implemented) and the
  # full Type=notify-reload lifecycle (reload via SIGHUP, then a clean SIGTERM
  # stop that runs the unit's `trap leave` handler and exits 109 -- upstream
  # TEST-59-RELOADING-RESTART.sh notify-reload subtest).
  #
  # Still skipped: reload rate limiting (ReloadLimitBurst not implemented) and
  # RestartMode=debug (not implemented).
  #
  # The notify-reload subtest exercises the graceful-SIGTERM stop fix: kill()
  # sends SIGTERM to the main process and waits up to TimeoutStopSec before
  # run_poststop's SIGKILL fallback, matching systemd's
  # ExecStop -> SIGTERM -> TimeoutStopSec -> SIGKILL -> ExecStopPost sequence.
  # Previously run_poststop's kill_all_remaining_processes SIGKILLed the main
  # process immediately, so the `trap leave SIGTERM` handler never ran and the
  # unit exited by signal (ExecMainStatus=9) instead of 109.
  #
  # THIS TEST CURRENTLY FAILS, measured 2026-07-28. Everything up to the
  # notify-reload subtest passes; it dies on the last assertion,
  #
  #     test "$(systemctl show -p ExecMainStatus --value notify-reload-test)" = 109
  #
  # with ExecMainStatus reading back EMPTY, not 109 and not the old 9. So the
  # graceful-SIGTERM half really did land -- terminate_gracefully exists at
  # services.rs:1272 and kill() calls it at 1377 -- but it does not help this
  # unit, and WHICH branch swallows it is not yet known.
  #
  # Do not assume it is the no-live-PID early return at services.rs:1289. The
  # lookup just above it (1281-1287) already falls back to scanning the live
  # PID table precisely because "pid/main_pid ... are cleared on some paths
  # before the stop (e.g. a transient notify unit)", which is this case, so
  # that hole looks covered. The other candidates are the ServiceExited/None
  # check at 1295, the KillMode::None guard at 1273 (the unit sets
  # KillMode=process, so this should not fire), and everything after the
  # SIGTERM is actually sent.
  #
  # Settling it needs a kmsg probe at terminate_gracefully's entry recording
  # main_pid, pid, and the unit's pid_table entry -- log:: never reaches the
  # console from PID 1, only crate::entrypoints::kmsg. An earlier attempt at
  # this bug burned several cycles on unconfirmed theories, so probe first and
  # do not patch on a hunch.
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

    : "Type=notify-reload full lifecycle: reload (SIGHUP=+11) then clean stop"
    : "(SIGTERM->leave=+7, +3 final = 109) exercises the graceful-SIGTERM fix"
    cat >/run/notify-reload-test.sh <<EOF
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    EXIT_STATUS=88
    LEAVE=0

    function reload() {
        systemd-notify --reloading --status="Adding 11 to exit status"
        EXIT_STATUS=\$((EXIT_STATUS + 11))
        systemd-notify --ready --status="Back running"
    }

    function leave() {
        systemd-notify --stopping --status="Adding 7 to exit status"
        EXIT_STATUS=\$((EXIT_STATUS + 7))
        LEAVE=1
        return 0
    }

    trap reload SIGHUP
    trap leave SIGTERM

    systemd-notify --ready
    systemd-notify --status="Running now"

    while [ \$LEAVE = 0 ] ; do
        sleep 1
    done

    systemd-notify --status="Adding 3 to exit status"
    EXIT_STATUS=\$((EXIT_STATUS + 3))
    exit \$EXIT_STATUS
    EOF
    chmod +x /run/notify-reload-test.sh

    systemd-run --unit notify-reload-test -p Type=notify-reload -p KillMode=process /run/notify-reload-test.sh
    systemctl reload notify-reload-test
    systemctl stop notify-reload-test
    test "$(systemctl show -p ExecMainStatus --value notify-reload-test)" = 109
    systemctl reset-failed notify-reload-test

    rm -f /run/systemd/system/testservice-*-59.service
    systemctl daemon-reload

    touch /testok
    TESTEOF
        chmod +x TEST-59-RELOADING-RESTART.sh
  '';
}
