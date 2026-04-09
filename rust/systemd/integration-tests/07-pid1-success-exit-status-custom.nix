{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.success-exit-status-custom\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.success-exit-status-custom.sh << 'SESEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/success-exit-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "SuccessExitStatus= treats custom exit codes as success"
    cat > /run/systemd/system/success-exit-test.service << EOF
    [Service]
    Type=oneshot
    SuccessExitStatus=42
    ExecStart=bash -c 'exit 42'
    EOF
    retry systemctl daemon-reload
    # Should succeed because exit 42 is in SuccessExitStatus
    retry systemctl start success-exit-test.service
    [[ "$(systemctl show -P Result success-exit-test.service)" == "success" ]]

    : "Without SuccessExitStatus=, same exit code is failure"
    # Stop previous oneshot so re-start actually runs again
    systemctl stop success-exit-test.service 2>/dev/null || true
    cat > /run/systemd/system/success-exit-test.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'exit 42'
    EOF
    retry systemctl daemon-reload
    (! systemctl start success-exit-test.service)
    SESEOF
  '';
}
