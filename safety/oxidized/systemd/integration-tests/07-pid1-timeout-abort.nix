{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.timeout-abort\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.timeout-abort.sh << 'TAEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="tabort-$RANDOM"
    HELPER="/run/tabort-helper.sh"
    BASH="$(command -v bash)"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "$HELPER"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # A service that IGNORES its watchdog signal (SIGABRT), so the abort timeout is
    # the only thing that can terminate it.
    cat > "$HELPER" << 'HEOF'
    #!/usr/bin/env bash
    trap ':' ABRT
    while :; do sleep 1; done
    HEOF
    chmod 755 "$HELPER"

    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=simple
    ExecStart=$BASH $HELPER
    WatchdogSec=2
    TimeoutStopSec=60
    TimeoutAbortSec=2
    EOF
    systemctl daemon-reload

    : "TimeoutAbortSec= SIGKILLs a watchdog-aborted service that ignores the signal"
    systemctl start "$UNIT.service"
    # Watchdog fires ~2s (SIGABRT, ignored); TimeoutAbortSec=2 -> SIGKILL ~4s.
    # Without the abort escalation the service is re-signaled forever (TimeoutStopSec
    # is not the abort timeout) and never dies, so this wait would time out.
    timeout 25 bash -c 'while [ "$(systemctl is-active "'"$UNIT"'.service")" != failed ]; do sleep 0.5; done'
    test "$(systemctl show -P Result "$UNIT.service")" = watchdog
    TAEOF
  '';
}
