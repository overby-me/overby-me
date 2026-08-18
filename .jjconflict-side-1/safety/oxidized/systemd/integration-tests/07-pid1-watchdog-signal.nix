{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.watchdog-signal\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.watchdog-signal.sh << 'WDSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    KUNIT="wdsig-kill-$RANDOM"
    AUNIT="wdsig-abrt-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$KUNIT.service" "$AUNIT.service" 2>/dev/null
        systemctl reset-failed "$KUNIT.service" "$AUNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$KUNIT.service" "/run/systemd/system/$AUNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    wait_watchdog() {
        local unit="$1" r
        for _ in $(seq 1 20); do
            r="$(systemctl show -P Result "$unit")"
            [[ "$r" == "watchdog" ]] && return 0
            sleep 1
        done
        echo "timed out waiting for Result=watchdog on $unit" >&2
        return 1
    }

    # WatchdogSignal=SIGKILL: the watchdog kill must use SIGKILL, so the reaped
    # main process reports signal 9 (a hardcoded SIGABRT would report 6).
    cat > "/run/systemd/system/$KUNIT.service" << EOF
    [Service]
    ExecStart=sleep infinity
    WatchdogSec=2
    WatchdogSignal=SIGKILL
    EOF

    # Default WatchdogSignal is SIGABRT -> signal 6.
    cat > "/run/systemd/system/$AUNIT.service" << EOF
    [Service]
    ExecStart=sleep infinity
    WatchdogSec=2
    EOF
    systemctl daemon-reload

    : "WatchdogSignal=SIGKILL kills the service with SIGKILL (ExecMainStatus=9)"
    systemctl start "$KUNIT.service"
    wait_watchdog "$KUNIT.service"
    test "$(systemctl show -P ExecMainStatus "$KUNIT.service")" = "9"

    : "Default WatchdogSignal is SIGABRT (ExecMainStatus=6)"
    systemctl start "$AUNIT.service"
    wait_watchdog "$AUNIT.service"
    test "$(systemctl show -P ExecMainStatus "$AUNIT.service")" = "6"
    WDSEOF
  '';
}
