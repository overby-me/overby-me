{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.refuse-manual\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.refuse-manual.sh << 'RMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        rm -f /run/systemd/system/refuse-start-test.service /run/systemd/system/refuse-stop-test.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "RefuseManualStart=yes refuses a manual systemctl start"
    cat > /run/systemd/system/refuse-start-test.service << EOF
    [Unit]
    RefuseManualStart=yes
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=true
    EOF
    systemctl daemon-reload
    # A directly-requested start must be refused; the unit stays inactive.
    (! systemctl start refuse-start-test.service 2>/dev/null)
    (! systemctl is-active refuse-start-test.service)

    : "RefuseManualStop=yes refuses a manual systemctl stop"
    cat > /run/systemd/system/refuse-stop-test.service << EOF
    [Unit]
    RefuseManualStop=yes
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=true
    EOF
    systemctl daemon-reload
    systemctl start refuse-stop-test.service
    systemctl is-active refuse-stop-test.service
    # A directly-requested stop must be refused; the unit stays active.
    (! systemctl stop refuse-stop-test.service 2>/dev/null)
    systemctl is-active refuse-stop-test.service
    RMEOF
  '';
}
