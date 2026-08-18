{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.umask\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.umask.sh << 'UMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/umask-test.service
        rm -f /tmp/umask-test-out /tmp/umask-test-file
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "UMask= sets process umask"
    cat > /run/systemd/system/umask-test.service << EOF
    [Service]
    Type=oneshot
    UMask=0077
    ExecStart=bash -c 'touch /tmp/umask-test-file && stat -c %%a /tmp/umask-test-file > /tmp/umask-test-out'
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/umask-test-file /tmp/umask-test-out
    retry systemctl start umask-test.service
    # With UMask=0077, new files should be 600 (rw-------)
    [[ "$(cat /tmp/umask-test-out)" == "600" ]]
    UMEOF
  '';
}
