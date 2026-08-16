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

    : "Multiple ExecStart= with - prefix continues past a failed command"
    cat > /run/systemd/system/multi-exec-dash.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'echo a >> /tmp/multi-exec-dash-log'
    ExecStart=-false
    ExecStart=bash -c 'echo c >> /tmp/multi-exec-dash-log'
    EOF
    rm -f /tmp/multi-exec-dash-log
    systemctl daemon-reload
    systemctl start multi-exec-dash.service
    systemctl is-active multi-exec-dash.service
    # The '-' prefix swallows false's failure, so the third command still runs;
    # exactly the a and c lines land, and nothing from the ignored false.
    grep -qx a /tmp/multi-exec-dash-log
    grep -qx c /tmp/multi-exec-dash-log
    [[ "$(wc -l < /tmp/multi-exec-dash-log)" -eq 2 ]]
    systemctl stop multi-exec-dash.service
    MESEOF
  '';
}
