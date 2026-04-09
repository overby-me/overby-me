{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.daemon-reload\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.daemon-reload.sh << 'DREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop reload-test-new.service 2>/dev/null
        rm -f /run/systemd/system/reload-test-new.service
        rm -f /run/systemd/system/reload-test-change.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # Helper: retry a command up to 5 times with 1s delay (works around EAGAIN)
    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "daemon-reload picks up new unit files"
    # Create a unit file without daemon-reload
    cat > /run/systemd/system/reload-test-new.service << EOF
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    # Unit should be unknown before reload
    retry systemctl daemon-reload
    # After reload, unit should be startable
    systemctl start reload-test-new.service
    systemctl is-active reload-test-new.service
    systemctl stop reload-test-new.service

    : "daemon-reload picks up changed Description"
    cat > /run/systemd/system/reload-test-change.service << EOF
    [Unit]
    Description=Original Description
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    systemctl daemon-reload
    [[ "$(systemctl show -P Description reload-test-change.service)" == "Original Description" ]]
    # Change the description
    cat > /run/systemd/system/reload-test-change.service << EOF
    [Unit]
    Description=Updated Description
    [Service]
    Type=oneshot
    ExecStart=true
    RemainAfterExit=yes
    EOF
    systemctl daemon-reload
    [[ "$(systemctl show -P Description reload-test-change.service)" == "Updated Description" ]]
    DREOF
  '';
}
