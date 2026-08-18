{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.removeipc-dynamic\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.removeipc-dynamic.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="dynipc-$RANDOM"
    SHM="/dev/shm/dynipc-test-$RANDOM"
    HELPER="/run/dynipc-helper.sh"
    BASH="$(command -v bash)"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "$HELPER" "$SHM"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    rm -f "$SHM"

    cat > "$HELPER" << 'HEOF'
    #!/usr/bin/env bash
    touch "$1"
    exec sleep infinity
    HEOF
    chmod 755 "$HELPER"

    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=simple
    DynamicUser=yes
    RemoveIPC=yes
    ExecStart=$BASH $HELPER $SHM
    EOF
    systemctl daemon-reload

    : "a DynamicUser= service creates a /dev/shm object owned by its dynamic UID"
    systemctl start "$UNIT.service"
    for _ in $(seq 1 40); do [ -e "$SHM" ] && break; sleep 0.25; done
    test -e "$SHM"
    OWNER="$(stat -c %u "$SHM")"
    test "$OWNER" -ge 61184
    test "$OWNER" -le 65519

    : "RemoveIPC=yes removes the DynamicUser= UID's IPC on stop"
    systemctl stop "$UNIT.service"
    for _ in $(seq 1 40); do [ ! -e "$SHM" ] && break; sleep 0.25; done
    test ! -e "$SHM"
    RIDEOF
  '';
}
