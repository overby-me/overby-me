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
    UNIT_MNT="scfmnt-$RANDOM"
    UNIT_ALLOW="scfallow-$RANDOM"
    OUT="/run/scf-out.$RANDOM"
    OUT_OK="/run/scfok-out.$RANDOM"
    OUT_MNT="/run/scfmnt-out.$RANDOM"
    OUT_ALLOW="/run/scfallow-out.$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" "$UNIT_OK.service" "$UNIT_MNT.service" "$UNIT_ALLOW.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" "$UNIT_OK.service" "$UNIT_MNT.service" "$UNIT_ALLOW.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "/run/systemd/system/$UNIT_OK.service" \
              "/run/systemd/system/$UNIT_MNT.service" "/run/systemd/system/$UNIT_ALLOW.service" \
              "$OUT" "$OUT_OK" "$OUT_MNT" "$OUT_ALLOW"
        umount /run/scf-mnt-dir /run/scf-allow-mnt 2>/dev/null
        rm -rf /run/scf-dir /run/scf-mnt-dir /run/scf-allow-mnt
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

    : "SystemCallFilter=~@mount blocks the mount() family via @group resolution"
    mkdir -p /run/scf-mnt-dir
    cat > "/run/systemd/system/$UNIT_MNT.service" << EOF
    [Service]
    Type=oneshot
    SystemCallFilter=~@mount
    SystemCallErrorNumber=EPERM
    ExecStart=/bin/sh -c 'if mount -t tmpfs tmpfs /run/scf-mnt-dir 2>/dev/null; then echo NOT-BLOCKED; else echo BLOCKED; fi > $OUT_MNT'
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT_MNT.service"
    cat "$OUT_MNT"
    grep -qx BLOCKED "$OUT_MNT"
    ! mountpoint -q /run/scf-mnt-dir

    : "allow-list SystemCallFilter=@system-service runs normally but blocks mount() (not in the set)"
    mkdir -p /run/scf-allow-mnt
    cat > "/run/systemd/system/$UNIT_ALLOW.service" << EOF
    [Service]
    Type=oneshot
    SystemCallFilter=@system-service
    SystemCallErrorNumber=EPERM
    ExecStart=/bin/sh -c 'echo alive; if mount -t tmpfs tmpfs /run/scf-allow-mnt 2>/dev/null; then echo MOUNT-OK; else echo MOUNT-BLOCKED; fi > $OUT_ALLOW'
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT_ALLOW.service"
    cat "$OUT_ALLOW"
    grep -qx MOUNT-BLOCKED "$OUT_ALLOW"
    ! mountpoint -q /run/scf-allow-mnt

    : "SystemCallArchitectures=native does not break a normal (native) service"
    UNIT_ARCH="scfarch-$RANDOM"
    OUT_ARCH="/run/scfarch-out.$RANDOM"
    cat > "/run/systemd/system/$UNIT_ARCH.service" << EOF
    [Service]
    Type=oneshot
    SystemCallArchitectures=native
    ExecStart=/bin/sh -c 'echo arch-native-ran > $OUT_ARCH'
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT_ARCH.service"
    grep -qx arch-native-ran "$OUT_ARCH"
    systemctl stop "$UNIT_ARCH.service" 2>/dev/null || true
    systemctl reset-failed "$UNIT_ARCH.service" 2>/dev/null || true
    rm -f "/run/systemd/system/$UNIT_ARCH.service" "$OUT_ARCH"
    RIDEOF
  '';
}
