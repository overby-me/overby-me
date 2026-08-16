{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.watchdog-usec\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.watchdog-usec.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="wdusec-$RANDOM"
    HELPER="/run/wdusec-helper.sh"
    OUT="/run/wdusec-out.$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "$HELPER" "$OUT"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # The service records its watchdog environment. WATCHDOG_PID must equal the
    # exec'd process' own PID ($$) — sd_watchdog_enabled() returns 0 otherwise.
    cat > "$HELPER" << 'HEOF'
    #!/usr/bin/env bash
    {
        echo "USEC=$WATCHDOG_USEC"
        if [ "$WATCHDOG_PID" = "$$" ]; then
            echo "PIDMATCH=yes"
        else
            echo "PIDMATCH=no($WATCHDOG_PID vs $$)"
        fi
    } > "$1"
    HEOF
    chmod 755 "$HELPER"

    : "WatchdogSec= exports WATCHDOG_USEC (interval in usec) + a matching WATCHDOG_PID"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    WatchdogSec=5s
    ExecStart=$HELPER $OUT
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"

    cat "$OUT"
    : "WatchdogSec=5s -> WATCHDOG_USEC=5000000"
    grep -qx 'USEC=5000000' "$OUT"
    : "WATCHDOG_PID is stamped with the service PID"
    grep -qx 'PIDMATCH=yes' "$OUT"

    : "a service without WatchdogSec= gets no WATCHDOG_USEC"
    UNIT2="nowd-$RANDOM"
    OUT2="/run/nowd-out.$RANDOM"
    cat > "/run/systemd/system/$UNIT2.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=$HELPER $OUT2
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT2.service"
    cat "$OUT2"
    grep -qx 'USEC=' "$OUT2"
    systemctl reset-failed "$UNIT2.service" 2>/dev/null || true
    rm -f "/run/systemd/system/$UNIT2.service" "$OUT2"
    RIDEOF
  '';
}
