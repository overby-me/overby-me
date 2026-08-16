{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.timeout-stop\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.timeout-stop.sh << 'TSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/timeout-stop-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "TimeoutStopSec= kills service after timeout"
    cat > /run/systemd/system/timeout-stop-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    TimeoutStopSec=2
    EOF
    retry systemctl daemon-reload
    retry systemctl start timeout-stop-test.service
    sleep 1
    systemctl is-active timeout-stop-test.service
    systemctl stop timeout-stop-test.service
    (! systemctl is-active timeout-stop-test.service)

    : "TimeoutStopSec= SIGKILLs a main process that ignores SIGTERM"
    cat > /run/systemd/system/timeout-stop-ignore.service << EOF
    [Service]
    ExecStart=bash -c 'trap "" TERM; while :; do sleep 1; done'
    TimeoutStopSec=2
    EOF
    retry systemctl daemon-reload
    retry systemctl start timeout-stop-ignore.service
    sleep 1
    systemctl is-active timeout-stop-ignore.service
    # The main process ignores SIGTERM, so the only way stop can complete is
    # the SIGKILL fired after the TimeoutStopSec window; systemctl stop blocks
    # until then. (The plain sleep case above dies on SIGTERM immediately and
    # never reaches the timeout path.)
    systemctl stop timeout-stop-ignore.service
    (! systemctl is-active timeout-stop-ignore.service)
    TSEOF
  '';
}
