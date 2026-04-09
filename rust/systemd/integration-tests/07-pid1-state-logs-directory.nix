{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.state-logs-directory\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.state-logs-directory.sh << 'SLDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop state-dir-test.service 2>/dev/null
        rm -f /run/systemd/system/state-dir-test.service
        rm -rf /var/lib/state-dir-test /var/log/log-dir-test /var/cache/cache-dir-test
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "StateDirectory= creates /var/lib/<name>"
    cat > /run/systemd/system/state-dir-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    StateDirectory=state-dir-test
    ExecStart=bash -c 'touch /var/lib/state-dir-test/marker'
    EOF
    retry systemctl daemon-reload
    retry systemctl start state-dir-test.service
    [[ -d /var/lib/state-dir-test ]]
    [[ -f /var/lib/state-dir-test/marker ]]
    systemctl stop state-dir-test.service

    : "LogsDirectory= creates /var/log/<name>"
    cat > /run/systemd/system/state-dir-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    LogsDirectory=log-dir-test
    ExecStart=bash -c 'touch /var/log/log-dir-test/marker'
    EOF
    retry systemctl daemon-reload
    retry systemctl start state-dir-test.service
    [[ -d /var/log/log-dir-test ]]
    [[ -f /var/log/log-dir-test/marker ]]
    systemctl stop state-dir-test.service

    : "CacheDirectory= creates /var/cache/<name>"
    cat > /run/systemd/system/state-dir-test.service << EOF
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    CacheDirectory=cache-dir-test
    ExecStart=bash -c 'touch /var/cache/cache-dir-test/marker'
    EOF
    retry systemctl daemon-reload
    retry systemctl start state-dir-test.service
    [[ -d /var/cache/cache-dir-test ]]
    [[ -f /var/cache/cache-dir-test/marker ]]
    systemctl stop state-dir-test.service
    SLDEOF
  '';
}
