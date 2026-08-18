{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.socket-rdpreserve\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.socket-rdpreserve.sh << 'SRPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    KEEP="rdps-keep-$RANDOM"
    DROP="rdps-drop-$RANDOM"
    KRD="/run/$KEEP-rd"
    DRD="/run/$DROP-rd"

    at_exit() {
        set +e
        systemctl stop "$KEEP.socket" "$DROP.socket" 2>/dev/null
        systemctl reset-failed "$KEEP.socket" "$DROP.socket" 2>/dev/null
        rm -rf "/run/systemd/system/$KEEP.socket" "/run/systemd/system/$KEEP.service" \
               "/run/systemd/system/$DROP.socket" "/run/systemd/system/$DROP.service" \
               "$KRD" "$DRD" "/run/$KEEP.sock" "/run/$DROP.sock"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # A socket whose RuntimeDirectory= must survive stop (Preserve=yes).
    cat > "/run/systemd/system/$KEEP.socket" << EOF
    [Socket]
    ListenStream=/run/$KEEP.sock
    RuntimeDirectory=$KEEP-rd
    RuntimeDirectoryPreserve=yes
    EOF
    cat > "/run/systemd/system/$KEEP.service" << EOF
    [Service]
    ExecStart=true
    EOF

    # A socket with the default preserve (no) — its dir must be removed on stop.
    cat > "/run/systemd/system/$DROP.socket" << EOF
    [Socket]
    ListenStream=/run/$DROP.sock
    RuntimeDirectory=$DROP-rd
    EOF
    cat > "/run/systemd/system/$DROP.service" << EOF
    [Service]
    ExecStart=true
    EOF
    systemctl daemon-reload

    : "RuntimeDirectoryPreserve=yes keeps a socket's RuntimeDirectory across stop"
    systemctl start "$KEEP.socket"
    test -d "$KRD"
    systemctl stop "$KEEP.socket"
    test -d "$KRD"

    : "default RuntimeDirectoryPreserve still removes a socket's RuntimeDirectory on stop"
    systemctl start "$DROP.socket"
    test -d "$DRD"
    systemctl stop "$DROP.socket"
    test ! -d "$DRD"
    SRPEOF
  '';
}
