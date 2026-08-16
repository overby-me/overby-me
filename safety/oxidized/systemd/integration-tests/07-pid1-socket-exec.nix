{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.socket-exec\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.socket-exec.sh << 'SXEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="sockexec-$RANDOM"
    FAILU="sockexecfail-$RANDOM"
    PRE="/run/sockexec-pre-$RANDOM"
    POST="/run/sockexec-post-$RANDOM"
    STOPPOST="/run/sockexec-stoppost-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.socket" "$FAILU.socket" 2>/dev/null
        systemctl reset-failed "$UNIT.socket" "$FAILU.socket" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.socket" "/run/systemd/system/$UNIT.service" \
              "/run/systemd/system/$FAILU.socket" "/run/systemd/system/$FAILU.service" \
              "$PRE" "$POST" "$STOPPOST" "/run/$UNIT.sock" "/run/$FAILU.sock"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    rm -f "$PRE" "$POST" "$STOPPOST"

    cat > "/run/systemd/system/$UNIT.socket" << EOF
    [Socket]
    ListenStream=/run/$UNIT.sock
    ExecStartPre=touch $PRE
    ExecStartPost=touch $POST
    ExecStopPost=touch $STOPPOST
    EOF
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    ExecStart=true
    EOF
    systemctl daemon-reload

    : "ExecStartPre= and ExecStartPost= run on socket start"
    systemctl start "$UNIT.socket"
    test -f "$PRE"
    test -f "$POST"
    test ! -f "$STOPPOST"

    : "ExecStopPost= runs on socket stop"
    systemctl stop "$UNIT.socket"
    test -f "$STOPPOST"

    # A non-'-' ExecStartPre that fails must abort the socket start.
    cat > "/run/systemd/system/$FAILU.socket" << EOF
    [Socket]
    ListenStream=/run/$FAILU.sock
    ExecStartPre=false
    EOF
    cat > "/run/systemd/system/$FAILU.service" << EOF
    [Service]
    ExecStart=true
    EOF
    systemctl daemon-reload

    : "a failing ExecStartPre= aborts the socket start"
    if systemctl start "$FAILU.socket" 2>/dev/null; then
        echo "socket with failing ExecStartPre unexpectedly started" >&2
        exit 1
    fi
    SXEOF
  '';
}
