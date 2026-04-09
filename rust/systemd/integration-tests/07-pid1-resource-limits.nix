{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.resource-limits\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.resource-limits.sh << 'RLEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/rlimit-test.service
        rm -f /tmp/rlimit-test-out
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "LimitNOFILE= sets NOFILE rlimit"
    cat > /run/systemd/system/rlimit-test.service << EOF
    [Service]
    Type=oneshot
    LimitNOFILE=4096
    ExecStart=bash -c 'ulimit -n > /tmp/rlimit-test-out'
    EOF
    retry systemctl daemon-reload
    retry systemctl start rlimit-test.service
    [[ "$(cat /tmp/rlimit-test-out)" == "4096" ]]

    : "LimitNPROC= sets NPROC rlimit"
    cat > /run/systemd/system/rlimit-test.service << EOF
    [Service]
    Type=oneshot
    LimitNPROC=512
    ExecStart=bash -c 'ulimit -u > /tmp/rlimit-test-out'
    EOF
    retry systemctl daemon-reload
    retry systemctl start rlimit-test.service
    [[ "$(cat /tmp/rlimit-test-out)" == "512" ]]

    : "LimitCORE= sets CORE rlimit"
    cat > /run/systemd/system/rlimit-test.service << EOF
    [Service]
    Type=oneshot
    LimitCORE=0
    ExecStart=bash -c 'ulimit -c > /tmp/rlimit-test-out'
    EOF
    retry systemctl daemon-reload
    retry systemctl start rlimit-test.service
    [[ "$(cat /tmp/rlimit-test-out)" == "0" ]]
    RLEOF
  '';
}
