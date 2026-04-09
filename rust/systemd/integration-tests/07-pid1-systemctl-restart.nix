{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.systemctl-restart\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.systemctl-restart.sh << 'SREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/restart-cmd-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "systemctl restart replaces main process"
    cat > /run/systemd/system/restart-cmd-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    EOF
    retry systemctl daemon-reload
    retry systemctl start restart-cmd-test.service
    ORIG_PID="$(systemctl show -P MainPID restart-cmd-test.service)"
    [[ "$ORIG_PID" -gt 0 ]]
    systemctl restart restart-cmd-test.service
    systemctl is-active restart-cmd-test.service
    NEW_PID="$(systemctl show -P MainPID restart-cmd-test.service)"
    [[ "$NEW_PID" -gt 0 ]]
    [[ "$ORIG_PID" -ne "$NEW_PID" ]]
    systemctl stop restart-cmd-test.service
    SREOF
  '';
}
