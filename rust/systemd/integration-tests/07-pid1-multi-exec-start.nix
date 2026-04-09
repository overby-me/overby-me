{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.multi-exec-start\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.multi-exec-start.sh << 'MESEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/multi-exec-*.service
        rm -f /tmp/multi-exec-*
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "Multiple ExecStart= in oneshot runs sequentially"
    cat > /run/systemd/system/multi-exec-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'echo step1 >> /tmp/multi-exec-log'
    ExecStart=bash -c 'echo step2 >> /tmp/multi-exec-log'
    ExecStart=bash -c 'echo step3 >> /tmp/multi-exec-log'
    RemainAfterExit=yes
    EOF
    rm -f /tmp/multi-exec-log
    retry systemctl daemon-reload
    retry systemctl start multi-exec-test.service
    systemctl is-active multi-exec-test.service
    [[ "$(cat /tmp/multi-exec-log)" == "step1
    step2
    step3" ]]
    systemctl stop multi-exec-test.service

    : "Multiple ExecStart= stops on first failure"
    cat > /run/systemd/system/multi-exec-fail.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'echo ok >> /tmp/multi-exec-fail-log'
    ExecStart=false
    ExecStart=bash -c 'echo should-not-run >> /tmp/multi-exec-fail-log'
    EOF
    rm -f /tmp/multi-exec-fail-log
    systemctl daemon-reload
    systemctl start multi-exec-fail.service || true
    (! systemctl is-active multi-exec-fail.service)
    # Only first command should have run
    [[ "$(cat /tmp/multi-exec-fail-log)" == "ok" ]]
    MESEOF
  '';
}
