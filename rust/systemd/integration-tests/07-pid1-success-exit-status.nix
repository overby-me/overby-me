{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.success-exit-status\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.success-exit-status.sh << 'SESEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/success-exit-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "SuccessExitStatus= treats custom exit code as success"
    cat > /run/systemd/system/success-exit-42.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'exit 42'
    SuccessExitStatus=42
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start success-exit-42.service
    systemctl is-active success-exit-42.service
    [[ "$(systemctl show -P Result success-exit-42.service)" == "success" ]]
    systemctl stop success-exit-42.service

    : "Without SuccessExitStatus=, exit 42 is failure"
    cat > /run/systemd/system/success-exit-fail.service << EOF
    [Service]
    Type=oneshot
    ExecStart=bash -c 'exit 42'
    EOF
    systemctl daemon-reload
    systemctl start success-exit-fail.service || true
    (! systemctl is-active success-exit-fail.service)
    SESEOF
  '';
}
