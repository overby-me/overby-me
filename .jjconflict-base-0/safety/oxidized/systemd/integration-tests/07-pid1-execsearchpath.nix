{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.execsearchpath\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.execsearchpath.sh << 'ESPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    DIR="/run/esp-bin-$RANDOM"
    MARK="/run/esp-mark-$RANDOM"
    UNIT="esp-ok-$RANDOM"
    NEG="esp-neg-$RANDOM"

    at_exit() {
        set +e
        systemctl reset-failed "$UNIT.service" "$NEG.service" 2>/dev/null
        rm -rf "$DIR" "$MARK" \
               "/run/systemd/system/$UNIT.service" "/run/systemd/system/$NEG.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # A tool reachable ONLY via ExecSearchPath= (its dir is not on the default PATH).
    mkdir -p "$DIR"
    cat > "$DIR/mytool" << 'HEOF'
    #!/usr/bin/env bash
    echo ran > "$1"
    HEOF
    chmod 755 "$DIR/mytool"

    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    ExecSearchPath=$DIR
    ExecStart=mytool $MARK
    EOF

    cat > "/run/systemd/system/$NEG.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=mytool $MARK
    EOF
    systemctl daemon-reload

    : "ExecSearchPath= resolves a bare ExecStart= command name"
    systemctl start "$UNIT.service"
    test -f "$MARK"
    grep -qx ran "$MARK"

    : "without ExecSearchPath=, the same bare name is unresolvable (start fails)"
    rm -f "$MARK"
    if systemctl start "$NEG.service" 2>/dev/null; then
        echo "negative case: $NEG unexpectedly started" >&2
        exit 1
    fi
    test ! -f "$MARK"
    ESPEOF
  '';
}
