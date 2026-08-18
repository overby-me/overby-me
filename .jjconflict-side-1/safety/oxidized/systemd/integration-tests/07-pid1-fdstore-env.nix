{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.fdstore-env\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.fdstore-env.sh << 'FDEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="fdstore-env-$RANDOM"
    HELPER="/run/fdstore-env-helper.sh"
    RESULT="/run/fdstore-env-result"
    NORES="/run/fdstore-env-none"
    rm -f "$RESULT" "$NORES"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" "$UNIT-none.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" "$UNIT-none.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "/run/systemd/system/$UNIT-none.service" \
              "$HELPER" "$RESULT" "$NORES"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # The service writes its own $FDSTORE env value; single-quoted heredoc keeps
    # $FDSTORE literal so the service (not this script) expands it at run time.
    cat > "$HELPER" << 'HEOF'
    #!/usr/bin/env bash
    echo "''${FDSTORE-UNSET}" > "$1"
    HEOF
    chmod +x "$HELPER"

    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    FileDescriptorStoreMax=8
    ExecStart=bash $HELPER $RESULT
    EOF

    # A service without FileDescriptorStoreMax must NOT have FDSTORE set.
    cat > "/run/systemd/system/$UNIT-none.service" << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash $HELPER $NORES
    EOF
    systemctl daemon-reload

    : "FileDescriptorStoreMax=8 exports FDSTORE=8 to the service"
    systemctl start "$UNIT.service"
    test "$(cat "$RESULT")" = "8"

    : "A service without FileDescriptorStoreMax has no FDSTORE in its environment"
    systemctl start "$UNIT-none.service"
    test "$(cat "$NORES")" = "UNSET"
    FDEEOF
  '';
}
