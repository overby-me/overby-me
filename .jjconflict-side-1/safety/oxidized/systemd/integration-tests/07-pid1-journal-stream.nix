{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.journal-stream\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.journal-stream.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="jstream-$RANDOM"
    HELPER="/run/jstream-helper.sh"
    OUT="/run/jstream-out.$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "$HELPER" "$OUT"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # StandardOutput=journal => the service's stdout fd is journald's stream
    # socket, and JOURNAL_STREAM must carry that fd's <dev>:<inode>. Duplicate
    # fd 1 to fd 9 up front: both the > redirect and $() command substitution
    # below reassign fd 1, so fd 9 preserves the original journal socket to stat.
    cat > "$HELPER" << 'HEOF'
    #!/usr/bin/env bash
    exec 9>&1
    INO_FD1="$(stat -L -c '%i' /proc/self/fd/9)"
    {
        echo "JS=$JOURNAL_STREAM"
        echo "INO_FD1=$INO_FD1"
    } > "$1"
    HEOF
    chmod 755 "$HELPER"

    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    StandardOutput=journal
    ExecStart=$HELPER $OUT
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"

    cat "$OUT"
    JS="$(grep '^JS=' "$OUT" | cut -d= -f2-)"
    INO_FD1="$(grep '^INO_FD1=' "$OUT" | cut -d= -f2-)"
    echo "JOURNAL_STREAM=$JS  stdout-inode=$INO_FD1"

    : "JOURNAL_STREAM is set with <dev>:<inode> form"
    [[ "$JS" =~ ^[0-9]+:[0-9]+$ ]]
    : "the inode in JOURNAL_STREAM matches the service's stdout fd"
    test "''${JS##*:}" = "$INO_FD1"
    RIDEOF
  '';
}
