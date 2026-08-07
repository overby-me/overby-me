{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec-pid\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.exec-pid.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="execpid-$RANDOM"
    HELPER="/run/execpid-helper.sh"
    OUT="/run/execpid-out.$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "$HELPER" "$OUT"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # The exec'd process' own PID ($$) must equal SYSTEMD_EXEC_PID, which
    # systemd sets for every service so sd_notify() can reject notifications
    # forwarded from any other process.
    cat > "$HELPER" << 'HEOF'
    #!/usr/bin/env bash
    if [ "$SYSTEMD_EXEC_PID" = "$$" ]; then
        echo "MATCH=yes" > "$1"
    else
        echo "MATCH=no($SYSTEMD_EXEC_PID vs $$)" > "$1"
    fi
    HEOF
    chmod 755 "$HELPER"

    : "systemd exports SYSTEMD_EXEC_PID equal to the exec'd process PID"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=$HELPER $OUT
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"

    cat "$OUT"
    grep -qx 'MATCH=yes' "$OUT"
    RIDEOF
  '';
}
