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

    : "RestrictAddressFamilies=AF_UNIX permits AF_UNIX sockets but blocks AF_INET with EAFNOSUPPORT (97)"
    PY="$(command -v python3 || true)"
    if [ -n "$PY" ]; then
        PYHELPER="/run/scf-af-check.py"
        cat > "$PYHELPER" << 'PYEOF'
    import socket, sys
    socket.socket(socket.AF_UNIX, socket.SOCK_STREAM).close()
    try: socket.socket(socket.AF_INET, socket.SOCK_STREAM).close(); r = "INET-OK"
    except OSError as e: r = "INET-BLOCKED-%d" % e.errno
    open(sys.argv[1], "w").write(r)
    PYEOF
        UNIT_AF="scfaf-$RANDOM"
        OUT_AF="/run/scfaf-out.$RANDOM"
        cat > "/run/systemd/system/$UNIT_AF.service" << EOF
    [Service]
    Type=oneshot
    RestrictAddressFamilies=AF_UNIX
    ExecStart=$PY $PYHELPER $OUT_AF
    EOF
        systemctl daemon-reload
        systemctl start "$UNIT_AF.service"
        cat "$OUT_AF"
        grep -qx INET-BLOCKED-97 "$OUT_AF"
        systemctl stop "$UNIT_AF.service" 2>/dev/null || true
        systemctl reset-failed "$UNIT_AF.service" 2>/dev/null || true
        rm -f "/run/systemd/system/$UNIT_AF.service" "$OUT_AF" "$PYHELPER"
    fi

    : "RestrictNetworkInterfaces=eth0 blocks loopback (127.0.0.1 via lo) with EPERM, proving the cgroup/skb filter attached"
    PYN="$(command -v python3 || true)"
    if [ -n "$PYN" ]; then
        NETHELPER="/run/scf-net-check.py"
        cat > "$NETHELPER" << 'PYEOF'
    import socket, sys, errno
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try: s.sendto(b"x", ("127.0.0.1", 9)); r = "LO-SENT"
    except OSError as e: r = "LO-BLOCKED" if e.errno == errno.EPERM else "LO-ERR%d" % e.errno
    open(sys.argv[1], "w").write(r)
    PYEOF
        UNIT_NET="scfnet-$RANDOM"
        OUT_NET="/run/scfnet-out.$RANDOM"
        cat > "/run/systemd/system/$UNIT_NET.service" << EOF
    [Service]
    Type=oneshot
    RestrictNetworkInterfaces=eth0
    ExecStart=$PYN $NETHELPER $OUT_NET
    EOF
        systemctl daemon-reload
        systemctl start "$UNIT_NET.service"
        cat "$OUT_NET"
        grep -qx LO-BLOCKED "$OUT_NET"
        systemctl stop "$UNIT_NET.service" 2>/dev/null || true
        systemctl reset-failed "$UNIT_NET.service" 2>/dev/null || true
        rm -f "/run/systemd/system/$UNIT_NET.service" "$OUT_NET" "$NETHELPER"
    fi

    : "IPAddressDeny=127.0.0.0/8 blocks loopback (EPERM); IPAddressAllow=127.0.0.0/8 permits it"
    PYI="$(command -v python3 || true)"
    if [ -n "$PYI" ]; then
        IPHELPER="/run/scf-ip-check.py"
        cat > "$IPHELPER" << 'PYEOF'
    import socket, sys, errno
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try: s.sendto(b"x", ("127.0.0.1", 9)); r = "OK"
    except OSError as e: r = "BLOCKED" if e.errno == errno.EPERM else "ERR%d" % e.errno
    open(sys.argv[1], "w").write(r)
    PYEOF
        UNIT_IPD="scfipd-$RANDOM"
        OUT_IPD="/run/scfipd-out.$RANDOM"
        cat > "/run/systemd/system/$UNIT_IPD.service" << EOF
    [Service]
    Type=oneshot
    IPAddressDeny=127.0.0.0/8
    ExecStart=$PYI $IPHELPER $OUT_IPD
    EOF
        systemctl daemon-reload
        systemctl start "$UNIT_IPD.service"
        cat "$OUT_IPD"
        grep -qx BLOCKED "$OUT_IPD"

        UNIT_IPA="scfipa-$RANDOM"
        OUT_IPA="/run/scfipa-out.$RANDOM"
        cat > "/run/systemd/system/$UNIT_IPA.service" << EOF
    [Service]
    Type=oneshot
    IPAddressAllow=127.0.0.0/8
    ExecStart=$PYI $IPHELPER $OUT_IPA
    EOF
        systemctl daemon-reload
        systemctl start "$UNIT_IPA.service"
        cat "$OUT_IPA"
        grep -qx OK "$OUT_IPA"

        systemctl stop "$UNIT_IPD.service" "$UNIT_IPA.service" 2>/dev/null || true
        systemctl reset-failed "$UNIT_IPD.service" "$UNIT_IPA.service" 2>/dev/null || true
        rm -f "/run/systemd/system/$UNIT_IPD.service" "/run/systemd/system/$UNIT_IPA.service" \
              "$OUT_IPD" "$OUT_IPA" "$IPHELPER"
    fi

    : "IPAddressDeny=::1/128 blocks IPv6 loopback (EPERM); IPAddressAllow=::1/128 permits it"
    PYI6="$(command -v python3 || true)"
    if [ -n "$PYI6" ] && [ -e /proc/net/if_inet6 ]; then
        IP6HELPER="/run/scf-ip6-check.py"
        cat > "$IP6HELPER" << 'PYEOF'
    import socket, sys, errno
    s = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)
    try: s.sendto(b"x", ("::1", 9)); r = "OK"
    except OSError as e: r = "BLOCKED" if e.errno == errno.EPERM else "ERR%d" % e.errno
    open(sys.argv[1], "w").write(r)
    PYEOF
        UNIT_IP6D="scfip6d-$RANDOM"
        OUT_IP6D="/run/scfip6d-out.$RANDOM"
        cat > "/run/systemd/system/$UNIT_IP6D.service" << EOF
    [Service]
    Type=oneshot
    IPAddressDeny=::1/128
    ExecStart=$PYI6 $IP6HELPER $OUT_IP6D
    EOF
        systemctl daemon-reload
        systemctl start "$UNIT_IP6D.service"
        cat "$OUT_IP6D"
        grep -qx BLOCKED "$OUT_IP6D"

        UNIT_IP6A="scfip6a-$RANDOM"
        OUT_IP6A="/run/scfip6a-out.$RANDOM"
        cat > "/run/systemd/system/$UNIT_IP6A.service" << EOF
    [Service]
    Type=oneshot
    IPAddressAllow=::1/128
    ExecStart=$PYI6 $IP6HELPER $OUT_IP6A
    EOF
        systemctl daemon-reload
        systemctl start "$UNIT_IP6A.service"
        cat "$OUT_IP6A"
        grep -qx OK "$OUT_IP6A"

        systemctl stop "$UNIT_IP6D.service" "$UNIT_IP6A.service" 2>/dev/null || true
        systemctl reset-failed "$UNIT_IP6D.service" "$UNIT_IP6A.service" 2>/dev/null || true
        rm -f "/run/systemd/system/$UNIT_IP6D.service" "/run/systemd/system/$UNIT_IP6A.service" \
              "$OUT_IP6D" "$OUT_IP6A" "$IP6HELPER"
    fi
    RIDEOF
  '';
}
