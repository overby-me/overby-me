{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.exec-reload-failure\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.exec-reload-failure.sh << 'ERFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop reload-fail-test.service 2>/dev/null
        rm -f /run/systemd/system/reload-fail-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "Failing ExecReload= should not kill the service"
    cat > /run/systemd/system/reload-fail-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecReload=false
    EOF
    retry systemctl daemon-reload
    retry systemctl start reload-fail-test.service
    systemctl is-active reload-fail-test.service
    # The reload SHOULD fail
    (! systemctl reload reload-fail-test.service)
    # But the service should still be running
    systemctl is-active reload-fail-test.service

    : "ExecReload=- prefix ignores failure"
    cat > /run/systemd/system/reload-fail-test.service << EOF
    [Service]
    ExecStart=sleep infinity
    ExecReload=-false
    EOF
    retry systemctl daemon-reload
    retry systemctl start reload-fail-test.service
    # Reload should succeed despite false, because of - prefix
    systemctl reload reload-fail-test.service
    systemctl is-active reload-fail-test.service
    ERFEOF
  '';
}
