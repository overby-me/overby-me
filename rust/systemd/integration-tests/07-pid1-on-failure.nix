{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.on-failure\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.on-failure.sh << 'OFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/onfail-trigger.service
        rm -f /run/systemd/system/onfail-handler.service
        rm -f /tmp/onfail-handler-ran
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "OnFailure= triggers handler when service fails"
    cat > /run/systemd/system/onfail-handler.service << EOF
    [Service]
    Type=oneshot
    ExecStart=touch /tmp/onfail-handler-ran
    RemainAfterExit=yes
    EOF
    cat > /run/systemd/system/onfail-trigger.service << EOF
    [Unit]
    OnFailure=onfail-handler.service
    [Service]
    Type=oneshot
    ExecStart=false
    EOF
    retry systemctl daemon-reload
    rm -f /tmp/onfail-handler-ran
    systemctl start onfail-trigger.service || true
    # Wait for OnFailure handler to run
    timeout 15 bash -c 'until [[ -f /tmp/onfail-handler-ran ]]; do sleep 0.5; done'
    [[ -f /tmp/onfail-handler-ran ]]

    : "OnFailure= does NOT trigger on success"
    cat > /run/systemd/system/onfail-trigger.service << EOF
    [Unit]
    OnFailure=onfail-handler.service
    [Service]
    Type=oneshot
    ExecStart=true
    EOF
    systemctl daemon-reload
    rm -f /tmp/onfail-handler-ran
    systemctl start onfail-trigger.service
    sleep 2
    [[ ! -f /tmp/onfail-handler-ran ]]
    OFEOF
  '';
}
