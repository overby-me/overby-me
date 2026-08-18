{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.condition-kernel-version\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.condition-kernel-version.sh << 'CKVEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/cond-kver-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "ConditionKernelVersion=>=1.0 succeeds (any real kernel is >= 1.0)"
    cat > /run/systemd/system/cond-kver-met.service << EOF
    [Unit]
    ConditionKernelVersion=>=1.0
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-kver-met.service
    systemctl is-active cond-kver-met.service
    systemctl stop cond-kver-met.service

    : "ConditionKernelVersion=>=999.0 skips (no such kernel)"
    cat > /run/systemd/system/cond-kver-skip.service << EOF
    [Unit]
    ConditionKernelVersion=>=999.0
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    systemctl start cond-kver-skip.service || true
    (! systemctl is-active cond-kver-skip.service)

    : "ConditionKernelVersion=<999.0 succeeds"
    cat > /run/systemd/system/cond-kver-lt.service << EOF
    [Unit]
    ConditionKernelVersion=<999.0
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-kver-lt.service
    systemctl is-active cond-kver-lt.service
    systemctl stop cond-kver-lt.service

    : "ConditionKernelVersion=!>=999.0 succeeds (negated false condition)"
    cat > /run/systemd/system/cond-kver-neg.service << EOF
    [Unit]
    ConditionKernelVersion=!>=999.0
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-kver-neg.service
    systemctl is-active cond-kver-neg.service
    systemctl stop cond-kver-neg.service
    CKVEOF
  '';
}
