{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec-start-pre-post\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.exec-start-pre-post.sh << 'ESPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/exec-order-*.service
        rm -f /tmp/exec-order-*
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "ExecStartPre= runs before ExecStart="
    cat > /run/systemd/system/exec-order-pre.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStartPre=bash -c 'echo pre > /tmp/exec-order-pre'
    ExecStart=bash -c 'test -f /tmp/exec-order-pre && echo main > /tmp/exec-order-main'
    EOF
    systemctl daemon-reload
    systemctl start exec-order-pre.service
    systemctl is-active exec-order-pre.service
    [[ -f /tmp/exec-order-pre ]]
    [[ -f /tmp/exec-order-main ]]
    systemctl stop exec-order-pre.service
    rm -f /tmp/exec-order-pre /tmp/exec-order-main

    : "ExecStartPost= runs after ExecStart="
    cat > /run/systemd/system/exec-order-post.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=bash -c 'echo main > /tmp/exec-order-main2'
    ExecStartPost=bash -c 'test -f /tmp/exec-order-main2 && echo post > /tmp/exec-order-post'
    EOF
    systemctl daemon-reload
    systemctl start exec-order-post.service
    systemctl is-active exec-order-post.service
    [[ -f /tmp/exec-order-main2 ]]
    [[ -f /tmp/exec-order-post ]]
    systemctl stop exec-order-post.service
    rm -f /tmp/exec-order-main2 /tmp/exec-order-post

    : "Multiple ExecStartPre= commands run in order"
    cat > /run/systemd/system/exec-order-multi-pre.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStartPre=bash -c 'echo 1 >> /tmp/exec-order-seq'
    ExecStartPre=bash -c 'echo 2 >> /tmp/exec-order-seq'
    ExecStart=bash -c 'echo 3 >> /tmp/exec-order-seq'
    ExecStartPost=bash -c 'echo 4 >> /tmp/exec-order-seq'
    EOF
    rm -f /tmp/exec-order-seq
    systemctl daemon-reload
    systemctl start exec-order-multi-pre.service
    systemctl is-active exec-order-multi-pre.service
    [[ "$(cat /tmp/exec-order-seq)" == "$(printf '1\n2\n3\n4')" ]]
    systemctl stop exec-order-multi-pre.service
    rm -f /tmp/exec-order-seq

    : "ExecStartPre= failure prevents ExecStart="
    cat > /run/systemd/system/exec-order-pre-fail.service << EOF
    [Service]
    Type=oneshot
    ExecStartPre=false
    ExecStart=bash -c 'echo should-not-run > /tmp/exec-order-nope'
    EOF
    rm -f /tmp/exec-order-nope
    systemctl daemon-reload
    systemctl start exec-order-pre-fail.service || true
    [[ ! -f /tmp/exec-order-nope ]]

    : "ExecStartPre= with - prefix ignores failure"
    cat > /run/systemd/system/exec-order-pre-dash.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStartPre=-false
    ExecStart=bash -c 'echo ran > /tmp/exec-order-dash'
    EOF
    rm -f /tmp/exec-order-dash
    systemctl daemon-reload
    systemctl start exec-order-pre-dash.service
    systemctl is-active exec-order-pre-dash.service
    [[ -f /tmp/exec-order-dash ]]
    systemctl stop exec-order-pre-dash.service
    rm -f /tmp/exec-order-dash
    ESPEOF
  '';
}
