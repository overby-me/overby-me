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
    KMEOF
  '';
}
