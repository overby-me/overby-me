{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.coredump-result\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.coredump-result.sh << 'CDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    SEGV="coredump-segv-$RANDOM"
    KILL="coredump-kill-$RANDOM"
    HELPER="/tmp/segv-$RANDOM.sh"

    at_exit() {
        set +e
        systemctl stop "$SEGV.service" "$KILL.service" 2>/dev/null
        systemctl reset-failed "$SEGV.service" "$KILL.service" 2>/dev/null
        rm -f "/run/systemd/system/$SEGV.service" "/run/systemd/system/$KILL.service" "$HELPER"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # A dumping signal (SIGSEGV) with core dumps enabled -> Result=core-dump.
    # printf keeps $$ literal so bash expands it to its own PID at run time.
    printf '#!/usr/bin/env bash\nkill -SEGV $$\n' > "$HELPER"
    chmod +x "$HELPER"
    cat > "/run/systemd/system/$SEGV.service" << EOF
    [Service]
    Type=oneshot
    LimitCORE=infinity
    ExecStart=bash $HELPER
    EOF

    # A non-dumping signal (SIGKILL) -> Result=signal.
    cat > "/run/systemd/system/$KILL.service" << EOF
    [Service]
    ExecStart=sleep infinity
    EOF
    systemctl daemon-reload

    : "A SIGSEGV death with core dumps enabled reports Result=core-dump"
    systemctl start "$SEGV.service" 2>/dev/null || true
    for _ in $(seq 1 50); do
        [[ "$(systemctl show -P Result "$SEGV.service")" != "" ]] && break
        sleep 0.2
    done
    result_segv="$(systemctl show -P Result "$SEGV.service")"
    test "$result_segv" = "core-dump"

    : "A SIGKILL death (no core) reports Result=signal, not core-dump"
    systemctl start "$KILL.service"
    systemctl is-active "$KILL.service"
    systemctl kill -s KILL "$KILL.service"
    for _ in $(seq 1 50); do
        [[ "$(systemctl is-active "$KILL.service" 2>/dev/null)" == "failed" ]] && break
        sleep 0.2
    done
    result_kill="$(systemctl show -P Result "$KILL.service")"
    test "$result_kill" = "signal"
    CDEOF
  '';
}
