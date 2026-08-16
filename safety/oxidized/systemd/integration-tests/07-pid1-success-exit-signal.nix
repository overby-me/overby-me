{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.success-exit-signal\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.success-exit-signal.sh << 'SESEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="success-exit-signal-$RANDOM"
    HELPER="/tmp/abort-$RANDOM.sh"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "$HELPER"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # The helper aborts itself with SIGABRT. $$ is the helper's own PID; never 0,
    # which would signal the whole process group (including the manager) and hang
    # it. Single-quoted printf keeps $$ literal so bash expands it at run time.
    printf '#!/usr/bin/env bash\nkill -s ABRT $$\n' > "$HELPER"
    chmod +x "$HELPER"

    # A Type=oneshot whose ExecStart dies from SIGABRT, but SIGABRT is declared a
    # success via SuccessExitStatus=. Before the fix the start wait judged the
    # signal death a failure (its success check looked only at exit codes) while
    # the exit handler judged it a success, so the unit was left stuck in Starting
    # and `systemctl start` hung forever.
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    SuccessExitStatus=SIGABRT
    ExecStart=bash $HELPER
    EOF
    systemctl daemon-reload

    : "systemctl start returns (does not hang) for a signal-success oneshot"
    timeout 30 systemctl start "$UNIT.service"

    : "Result is success because SIGABRT is in SuccessExitStatus="
    result="$(systemctl show -P Result "$UNIT.service")"
    test "$result" = "success"

    : "RemainAfterExit=yes keeps it active after the clean signal death"
    systemctl is-active "$UNIT.service"
    SESEOF
  '';
}
