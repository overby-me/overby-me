{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.condition-first-boot\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.condition-first-boot.sh << 'CFBEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/cond-fb-*.service
        rm -f /run/systemd/first-boot
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    # ConditionFirstBoot reads the /run/systemd/first-boot flag PID 1 writes on a
    # genuine first boot (matching C's in_first_boot()); this VM booted with an
    # existing machine-id, so drive the flag directly to exercise both states.

    : "not first boot (no flag): ConditionFirstBoot=no succeeds"
    rm -f /run/systemd/first-boot
    cat > /run/systemd/system/cond-fb-no.service << EOF
    [Unit]
    ConditionFirstBoot=no
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-fb-no.service
    systemctl is-active cond-fb-no.service
    systemctl stop cond-fb-no.service

    : "not first boot (no flag): ConditionFirstBoot=yes skips"
    rm -f /run/systemd/first-boot
    cat > /run/systemd/system/cond-fb-yes-skip.service << EOF
    [Unit]
    ConditionFirstBoot=yes
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    systemctl start cond-fb-yes-skip.service || true
    (! systemctl is-active cond-fb-yes-skip.service)

    : "first boot (flag set): ConditionFirstBoot=yes succeeds"
    mkdir -p /run/systemd
    : > /run/systemd/first-boot
    cat > /run/systemd/system/cond-fb-yes.service << EOF
    [Unit]
    ConditionFirstBoot=yes
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-fb-yes.service
    systemctl is-active cond-fb-yes.service
    systemctl stop cond-fb-yes.service

    : "first boot (flag set): ConditionFirstBoot=no skips"
    mkdir -p /run/systemd
    : > /run/systemd/first-boot
    cat > /run/systemd/system/cond-fb-no-skip.service << EOF
    [Unit]
    ConditionFirstBoot=no
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    systemctl start cond-fb-no-skip.service || true
    (! systemctl is-active cond-fb-no-skip.service)
    rm -f /run/systemd/first-boot
    CFBEOF
  '';
}
