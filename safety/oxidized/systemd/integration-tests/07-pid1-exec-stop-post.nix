{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec-stop-post\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.exec-stop-post.sh << 'ESPOSTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/stop-order-*.service
        rm -f /tmp/stop-order-*
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "ExecStop= runs on stop"
    cat > /run/systemd/system/stop-order-basic.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecStop=bash -c 'echo stopped > /tmp/stop-order-basic'
    EOF
    rm -f /tmp/stop-order-basic
    systemctl daemon-reload
    systemctl start stop-order-basic.service
    systemctl is-active stop-order-basic.service
    systemctl stop stop-order-basic.service
    [[ -f /tmp/stop-order-basic ]]
    [[ "$(cat /tmp/stop-order-basic)" == "stopped" ]]
    rm -f /tmp/stop-order-basic

    : "ExecStopPost= runs after service exits"
    cat > /run/systemd/system/stop-order-post.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecStopPost=bash -c 'echo post > /tmp/stop-order-post'
    EOF
    rm -f /tmp/stop-order-post
    systemctl daemon-reload
    systemctl start stop-order-post.service
    systemctl is-active stop-order-post.service
    systemctl stop stop-order-post.service
    [[ -f /tmp/stop-order-post ]]
    [[ "$(cat /tmp/stop-order-post)" == "post" ]]
    rm -f /tmp/stop-order-post

    : "ExecStopPost= runs even when ExecStop= fails"
    cat > /run/systemd/system/stop-order-post-after-fail.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecStop=false
    ExecStopPost=bash -c 'echo ran-anyway > /tmp/stop-order-post-fail'
    EOF
    rm -f /tmp/stop-order-post-fail
    systemctl daemon-reload
    systemctl start stop-order-post-after-fail.service
    systemctl is-active stop-order-post-after-fail.service
    # ExecStop=false fails, so systemctl stop may return non-zero
    systemctl stop stop-order-post-after-fail.service || true
    sleep 1
    [[ -f /tmp/stop-order-post-fail ]]
    [[ "$(cat /tmp/stop-order-post-fail)" == "ran-anyway" ]]
    rm -f /tmp/stop-order-post-fail

    : "ExecStop= and ExecStopPost= run in order"
    cat > /run/systemd/system/stop-order-sequence.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecStop=bash -c 'echo stop >> /tmp/stop-order-seq'
    ExecStopPost=bash -c 'echo post >> /tmp/stop-order-seq'
    EOF
    rm -f /tmp/stop-order-seq
    systemctl daemon-reload
    systemctl start stop-order-sequence.service
    systemctl is-active stop-order-sequence.service
    systemctl stop stop-order-sequence.service
    [[ "$(cat /tmp/stop-order-seq)" == "$(printf 'stop\npost')" ]]
    rm -f /tmp/stop-order-seq
    ESPOSTEOF
  '';
}
