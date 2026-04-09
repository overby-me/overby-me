{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.runtime-directory\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.runtime-directory.sh << 'RDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop runtime-dir-test.service 2>/dev/null
        rm -f /run/systemd/system/runtime-dir-test.service
        rm -rf /run/runtime-dir-test
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "RuntimeDirectory= creates directory on start"
    cat > /run/systemd/system/runtime-dir-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    RuntimeDirectory=runtime-dir-test
    ExecStart=bash -c 'touch /run/runtime-dir-test/marker'
    EOF
    retry systemctl daemon-reload
    retry systemctl start runtime-dir-test.service
    [[ -d /run/runtime-dir-test ]]
    [[ -f /run/runtime-dir-test/marker ]]

    : "RuntimeDirectory= removed on stop"
    systemctl stop runtime-dir-test.service
    [[ ! -d /run/runtime-dir-test ]]
    RDEOF
  '';
}
