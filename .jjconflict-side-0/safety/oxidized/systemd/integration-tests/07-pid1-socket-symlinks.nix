{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.socket-symlinks\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.socket-symlinks.sh << 'SSYEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="socksym-$RANDOM"
    RMUNIT="socksymrm-$RANDOM"
    SOCK="/run/$UNIT.sock"
    RMSOCK="/run/$RMUNIT.sock"
    LINK="/run/socksym-link-$RANDOM"
    RMLINK="/run/socksymrm-link-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.socket" "$RMUNIT.socket" 2>/dev/null
        systemctl reset-failed "$UNIT.socket" "$RMUNIT.socket" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.socket" "/run/systemd/system/$UNIT.service" \
              "/run/systemd/system/$RMUNIT.socket" "/run/systemd/system/$RMUNIT.service" \
              "$SOCK" "$RMSOCK" "$LINK" "$RMLINK"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    rm -f "$LINK" "$RMLINK"

    cat > "/run/systemd/system/$UNIT.socket" << EOF
    [Socket]
    ListenStream=$SOCK
    Symlinks=$LINK
    EOF
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    ExecStart=true
    EOF

    cat > "/run/systemd/system/$RMUNIT.socket" << EOF
    [Socket]
    ListenStream=$RMSOCK
    Symlinks=$RMLINK
    RemoveOnStop=yes
    EOF
    cat > "/run/systemd/system/$RMUNIT.service" << EOF
    [Service]
    ExecStart=true
    EOF
    systemctl daemon-reload

    : "Symlinks= creates a symlink pointing at the socket on start"
    systemctl start "$UNIT.socket"
    test -L "$LINK"
    test "$(readlink "$LINK")" = "$SOCK"

    : "without RemoveOnStop=, the Symlinks= symlink persists after stop"
    systemctl stop "$UNIT.socket"
    test -L "$LINK"

    : "RemoveOnStop=yes removes the Symlinks= symlink on stop"
    systemctl start "$RMUNIT.socket"
    test -L "$RMLINK"
    systemctl stop "$RMUNIT.socket"
    test ! -e "$RMLINK"
    SSYEOF
  '';
}
