{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.remain-after-exit\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.remain-after-exit.sh << 'RAEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop remain-test.service 2>/dev/null
        rm -f /run/systemd/system/remain-test.service
        rm -f /tmp/remain-stop-marker /tmp/remain-start-marker
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "RemainAfterExit=yes keeps service active after ExecStart finishes"
    cat > /run/systemd/system/remain-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'touch /tmp/remain-start-marker'
    ExecStop=bash -c 'touch /tmp/remain-stop-marker'
    EOF
    retry systemctl daemon-reload
    retry systemctl start remain-test.service
    [[ -f /tmp/remain-start-marker ]]
    # Service should still be active
    systemctl is-active remain-test.service

    : "ExecStop= runs when stopping RemainAfterExit service"
    systemctl stop remain-test.service
    [[ -f /tmp/remain-stop-marker ]]
    (! systemctl is-active remain-test.service)
    RAEEOF
  '';
}
