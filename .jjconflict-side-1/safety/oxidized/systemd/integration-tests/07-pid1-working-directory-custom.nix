{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.working-directory-custom\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.working-directory-custom.sh << 'WDCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/wd-test.service
        rm -f /tmp/wd-test-out
        rm -rf /tmp/wd-test-dir
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "WorkingDirectory= sets cwd for ExecStart"
    mkdir -p /tmp/wd-test-dir
    cat > /run/systemd/system/wd-test.service << EOF
    [Service]
    Type=oneshot
    WorkingDirectory=/tmp/wd-test-dir
    ExecStart=bash -c 'pwd > /tmp/wd-test-out'
    EOF
    retry systemctl daemon-reload
    retry systemctl start wd-test.service
    [[ "$(cat /tmp/wd-test-out)" == "/tmp/wd-test-dir" ]]

    WDCEOF
  '';
}
