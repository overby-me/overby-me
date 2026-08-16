{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.stop-when-unneeded\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.stop-when-unneeded.sh << 'SWUEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    KEEP="swu-keep-$RANDOM"
    H1="swu-h1-$RANDOM"
    H2="swu-h2-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$H1.service" "$H2.service" "$KEEP.service" 2>/dev/null
        systemctl reset-failed "$H1.service" "$H2.service" "$KEEP.service" 2>/dev/null
        rm -f "/run/systemd/system/$KEEP.service" \
              "/run/systemd/system/$H1.service" \
              "/run/systemd/system/$H2.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # KEEP opts into automatic GC when nothing needs it.
    cat > "/run/systemd/system/$KEEP.service" << EOF
    [Unit]
    StopWhenUnneeded=yes
    [Service]
    ExecStart=sleep infinity
    EOF

    # Two holders that pull KEEP in via Wants=.
    for H in "$H1" "$H2"; do
        cat > "/run/systemd/system/$H.service" << EOF
    [Unit]
    Wants=$KEEP.service
    After=$KEEP.service
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=true
    EOF
    done
    systemctl daemon-reload

    : "Starting a holder pulls in the StopWhenUnneeded= unit"
    systemctl start "$H1.service"
    systemctl is-active "$KEEP.service"

    : "A second holder also keeps it needed"
    systemctl start "$H2.service"
    systemctl is-active "$KEEP.service"

    : "Stopping one holder while another still needs it does NOT stop it"
    systemctl stop "$H1.service"
    systemctl is-active "$KEEP.service"

    : "Stopping the last holder auto-stops the now-unneeded unit"
    systemctl stop "$H2.service"
    # The GC is a deferred dispatcher event; give it and the long-running
    # ExecStart a moment to be reaped.
    for _ in $(seq 1 50); do
        systemctl is-active "$KEEP.service" >/dev/null 2>&1 || break
        sleep 0.2
    done
    (! systemctl is-active "$KEEP.service")
    SWUEOF
  '';
}
