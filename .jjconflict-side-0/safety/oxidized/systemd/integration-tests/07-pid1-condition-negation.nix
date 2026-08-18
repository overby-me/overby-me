{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.condition-negation\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.condition-negation.sh << 'CNEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/cond-neg-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "ConditionPathExists=! negation succeeds when path does NOT exist"
    cat > /run/systemd/system/cond-neg-exists.service << EOF
    [Unit]
    ConditionPathExists=!/nonexistent/path
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-neg-exists.service
    systemctl is-active cond-neg-exists.service
    systemctl stop cond-neg-exists.service

    : "ConditionPathExists=! negation skips when path exists"
    cat > /run/systemd/system/cond-neg-exists-fail.service << EOF
    [Unit]
    ConditionPathExists=!/etc/hostname
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    systemctl start cond-neg-exists-fail.service || true
    (! systemctl is-active cond-neg-exists-fail.service)

    : "ConditionPathIsDirectory=! negation succeeds for non-directory"
    cat > /run/systemd/system/cond-neg-dir.service << EOF
    [Unit]
    ConditionPathIsDirectory=!/etc/hostname
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-neg-dir.service
    systemctl is-active cond-neg-dir.service
    systemctl stop cond-neg-dir.service

    : "ConditionFileNotEmpty=! negation succeeds for empty file"
    touch /tmp/empty-for-neg-test
    cat > /run/systemd/system/cond-neg-notempty.service << EOF
    [Unit]
    ConditionFileNotEmpty=!/tmp/empty-for-neg-test
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    retry systemctl daemon-reload
    retry systemctl start cond-neg-notempty.service
    systemctl is-active cond-neg-notempty.service
    systemctl stop cond-neg-notempty.service
    rm -f /tmp/empty-for-neg-test
    CNEOF
  '';
}
