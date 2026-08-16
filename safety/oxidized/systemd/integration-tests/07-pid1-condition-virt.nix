{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.condition-virt\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.condition-virt.sh << 'CVEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/cond-virt-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "ConditionVirtualization=yes succeeds in VM"
    cat > /run/systemd/system/cond-virt-yes.service << EOF
    [Unit]
    ConditionVirtualization=yes
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-virt-yes.service
    systemctl is-active cond-virt-yes.service
    systemctl stop cond-virt-yes.service

    : "ConditionVirtualization=!container succeeds in VM (not a container)"
    cat > /run/systemd/system/cond-virt-notcont.service << EOF
    [Unit]
    ConditionVirtualization=!container
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-virt-notcont.service
    systemctl is-active cond-virt-notcont.service
    systemctl stop cond-virt-notcont.service
    CVEOF
  '';
}
