{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.removeipc\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.removeipc.sh << 'RIEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # Unique, otherwise-unused UIDs so cleaning their IPC nukes nothing else.
    RUID=54321
    CUID=54322
    UNIT="rmipc-$RANDOM"
    CTRL="rmipcctl-$RANDOM"
    RSHM="/dev/shm/rmipc-test-$RANDOM"
    CSHM="/dev/shm/rmipcctl-test-$RANDOM"
    HELPER="/run/rmipc-helper.sh"
    BASH="$(command -v bash)"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" "$CTRL.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" "$CTRL.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "/run/systemd/system/$CTRL.service" \
              "$HELPER" "$RSHM" "$CSHM"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    rm -f "$RSHM" "$CSHM"

    # Create a POSIX shm object (a file under /dev/shm, owned by our uid) then stay
    # running so the object persists until the service is stopped.
    cat > "$HELPER" << 'HEOF'
    #!/usr/bin/env bash
    touch "$1"
    exec sleep infinity
    HEOF
    chmod 755 "$HELPER"

    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=simple
    User=$RUID
    RemoveIPC=yes
    ExecStart=$BASH $HELPER $RSHM
    EOF
    cat > "/run/systemd/system/$CTRL.service" << EOF
    [Service]
    Type=simple
    User=$CUID
    ExecStart=$BASH $HELPER $CSHM
    EOF
    systemctl daemon-reload

    : "both services create a /dev/shm object owned by their uid"
    systemctl start "$UNIT.service" "$CTRL.service"
    for _ in $(seq 1 40); do [ -e "$RSHM" ] && [ -e "$CSHM" ] && break; sleep 0.25; done
    test "$(stat -c %u "$RSHM")" = "$RUID"
    test "$(stat -c %u "$CSHM")" = "$CUID"

    : "RemoveIPC=yes removes the uid's /dev/shm object on stop"
    systemctl stop "$UNIT.service"
    for _ in $(seq 1 40); do [ ! -e "$RSHM" ] && break; sleep 0.25; done
    test ! -e "$RSHM"

    : "without RemoveIPC=, the control's object survives stop"
    systemctl stop "$CTRL.service"
    sleep 1
    test -e "$CSHM"
    RIEOF
  '';
}
