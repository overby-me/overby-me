{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.assert\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.assert.sh << 'ASEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl reset-failed assert-test.service 2>/dev/null
        rm -f /run/systemd/system/assert-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "AssertPathExists= failure puts the unit in the failed state"
    cat > /run/systemd/system/assert-test.service << EOF
    [Unit]
    AssertPathExists=/nonexistent-assert-path-xyz
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=true
    EOF
    systemctl daemon-reload
    # An assertion failure is an ERROR: start fails and the unit is failed,
    # unlike a condition (which would be a silent skip).
    (! systemctl start assert-test.service 2>/dev/null)
    [[ "$(systemctl show -P ActiveState assert-test.service)" == "failed" ]]
    systemctl reset-failed assert-test.service

    : "ConditionPathExists= failure skips the unit (not failed)"
    cat > /run/systemd/system/assert-test.service << EOF
    [Unit]
    ConditionPathExists=/nonexistent-assert-path-xyz
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=true
    EOF
    systemctl daemon-reload
    # A condition failure is NOT an error: start succeeds, unit is skipped.
    systemctl start assert-test.service
    [[ "$(systemctl show -P ActiveState assert-test.service)" != "failed" ]]
    ASEOF
  '';
}
