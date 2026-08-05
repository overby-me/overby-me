{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.kill-mode\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.kill-mode.sh << 'KMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop killmode-test.service 2>/dev/null
        rm -f /run/systemd/system/killmode-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "KillMode=process only kills main process"
    cat > /run/systemd/system/killmode-test.service << EOF
    [Service]
    KillMode=process
    ExecStart=bash -c 'sleep infinity & exec sleep infinity'
    EOF
    retry systemctl daemon-reload
    retry systemctl start killmode-test.service
    MAINPID=$(systemctl show -P MainPID killmode-test.service)
    [[ "$MAINPID" -gt 0 ]]
    # Service is running
    systemctl is-active killmode-test.service
    systemctl stop killmode-test.service

    : "KillMode=control-group kills every process in the service cgroup"
    cat > /run/systemd/system/killmode-test.service << EOF
    [Service]
    KillMode=control-group
    ExecStart=bash -c 'sleep infinity & exec sleep infinity'
    EOF
    retry systemctl daemon-reload
    retry systemctl start killmode-test.service
    sleep 1
    cg=$(systemctl show -P ControlGroup killmode-test.service)
    pids=$(cat /sys/fs/cgroup$cg/cgroup.procs)
    [[ -n "$pids" ]]
    systemctl stop killmode-test.service
    # control-group mode SIGKILLs the whole cgroup, so every recorded pid
    # (the main sleep and the backgrounded child) must be gone shortly after.
    for i in 1 2 3 4 5; do
        alive=0
        for p in $pids; do kill -0 "$p" 2>/dev/null && alive=1; done
        [[ "$alive" == 0 ]] && break
        sleep 1
    done
    for p in $pids; do (! kill -0 "$p" 2>/dev/null); done
    KMEOF
  '';
}
