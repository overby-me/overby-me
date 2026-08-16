{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.timer-remain-after-elapse\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.timer-remain-after-elapse.sh << 'RAEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    NOREMAIN="timer-noremain-$RANDOM"
    REMAIN="timer-remain-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$NOREMAIN.timer" "$REMAIN.timer" 2>/dev/null
        systemctl reset-failed "$NOREMAIN.timer" "$REMAIN.timer" 2>/dev/null
        rm -f "/run/systemd/system/$NOREMAIN.timer" "/run/systemd/system/$REMAIN.timer" \
              "/run/systemd/system/$NOREMAIN.service" "/run/systemd/system/$REMAIN.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    for U in "$NOREMAIN" "$REMAIN"; do
        cat > "/run/systemd/system/$U.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=true
    EOF
    done

    # One-shot timer with RemainAfterElapse=no: must deactivate after it elapses.
    cat > "/run/systemd/system/$NOREMAIN.timer" << EOF
    [Timer]
    OnActiveSec=1s
    RemainAfterElapse=no
    EOF

    # One-shot timer with the default RemainAfterElapse=yes: stays active.
    cat > "/run/systemd/system/$REMAIN.timer" << EOF
    [Timer]
    OnActiveSec=1s
    EOF

    systemctl daemon-reload
    systemctl start "$NOREMAIN.timer" "$REMAIN.timer"

    : "RemainAfterElapse=no one-shot timer deactivates after elapsing"
    for _ in $(seq 1 60); do
        [[ "$(systemctl is-active "$NOREMAIN.timer" 2>/dev/null)" != "active" ]] && break
        sleep 0.2
    done
    (! systemctl is-active "$NOREMAIN.timer")

    : "Default (RemainAfterElapse=yes) one-shot timer stays active after elapsing"
    # Give it well past its 1s elapse, then confirm it is still active.
    sleep 3
    systemctl is-active "$REMAIN.timer"
    RAEEOF
  '';
}
