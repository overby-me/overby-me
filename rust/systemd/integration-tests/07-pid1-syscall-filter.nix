{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.syscall-filter\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.syscall-filter.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="scf-$RANDOM"
    UNIT_OK="scfok-$RANDOM"
    OUT="/run/scf-out.$RANDOM"
    OUT_OK="/run/scfok-out.$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" "$UNIT_OK.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" "$UNIT_OK.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "/run/systemd/system/$UNIT_OK.service" \
              "$OUT" "$OUT_OK"
        rm -rf /run/scf-dir
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "SystemCallFilter=~mkdir mkdirat with SystemCallErrorNumber=EPERM blocks mkdir"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    SystemCallFilter=~mkdir mkdirat
    SystemCallErrorNumber=EPERM
    ExecStart=/bin/sh -c 'if mkdir /run/scf-dir 2>/dev/null; then echo NOT-BLOCKED; else echo BLOCKED; fi > $OUT'
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    cat "$OUT"
    grep -qx BLOCKED "$OUT"
    test ! -e /run/scf-dir

    : "the same command WITHOUT the filter succeeds (proves the filter is what blocked it)"
    cat > "/run/systemd/system/$UNIT_OK.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=/bin/sh -c 'if mkdir /run/scf-dir 2>/dev/null; then echo NOT-BLOCKED; else echo BLOCKED; fi > $OUT_OK'
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT_OK.service"
    cat "$OUT_OK"
    grep -qx NOT-BLOCKED "$OUT_OK"
    test -d /run/scf-dir
    RIDEOF
  '';
}
