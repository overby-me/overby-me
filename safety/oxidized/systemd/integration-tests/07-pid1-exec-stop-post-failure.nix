{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec-stop-post-failure\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.exec-stop-post-failure.sh << 'ESPFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/stoppost-test.service
        rm -f /tmp/stoppost-marker
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "ExecStopPost= runs even when service fails"
    cat > /run/systemd/system/stoppost-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=false
    ExecStopPost=touch /tmp/stoppost-marker
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/stoppost-marker
    (! systemctl start stoppost-test.service)
    # ExecStopPost should have run despite failure
    sleep 1
    [[ -f /tmp/stoppost-marker ]]

    : "ExecStopPost= runs after normal stop"
    cat > /run/systemd/system/stoppost-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecStopPost=touch /tmp/stoppost-marker
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/stoppost-marker
    retry systemctl start stoppost-test.service
    systemctl stop stoppost-test.service
    sleep 1
    [[ -f /tmp/stoppost-marker ]]
    ESPFEOF
  '';
}
