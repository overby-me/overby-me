{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.guess-main-pid\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.guess-main-pid.sh << 'RIDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    UNIT="forkguess-$RANDOM"
    HELPER="/run/forkguess-helper.sh"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "$HELPER"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Type=forking parent: daemonize a child (sleep) and exit, leaving it as the
    # sole process in the service's cgroup. disown detaches it from the shell's
    # job table so no SIGHUP reaches it on the parent's exit.
    cat > "$HELPER" << 'HEOF'
    #!/usr/bin/env bash
    sleep infinity &
    disown
    exit 0
    HEOF
    chmod 755 "$HELPER"

    : "a Type=forking service with no PIDFile= daemonizes a process and exits"
    # With no PIDFile= and no sd_notify MAINPID, systemd must GUESS the main PID
    # (GuessMainPID=, default yes) as the sole process left in the cgroup.
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=forking
    ExecStart=$HELPER
    EOF
    systemctl daemon-reload

    systemctl start "$UNIT.service"

    : "GuessMainPID= populates a non-zero MainPID from the cgroup"
    MAINPID=""
    for _ in $(seq 1 40); do
        MAINPID="$(systemctl show -P MainPID "$UNIT.service")"
        [ -n "$MAINPID" ] && [ "$MAINPID" != "0" ] && break
        sleep 0.25
    done
    echo "guessed MainPID=$MAINPID"
    test -n "$MAINPID"
    test "$MAINPID" != "0"

    : "the guessed PID is the running sleep daemon left in the cgroup"
    test "$(cat "/proc/$MAINPID/comm")" = "sleep"
    RIDEOF
  '';
}
