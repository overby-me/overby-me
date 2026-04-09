{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.service-dependencies\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.service-dependencies.sh << 'SDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    at_exit() {
        set +e
        systemctl stop dep-*.service 2>/dev/null
        rm -f /run/systemd/system/dep-*.service
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "Wants= starts the wanted unit"
    printf '[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=true\n' > /run/systemd/system/dep-wanted.service
    printf '[Unit]\nWants=dep-wanted.service\nAfter=dep-wanted.service\n[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=true\n' > /run/systemd/system/dep-wanter.service
    systemctl daemon-reload
    systemctl start dep-wanter.service
    sleep 1
    systemctl is-active dep-wanted.service
    systemctl is-active dep-wanter.service
    systemctl stop dep-wanter.service dep-wanted.service

    : "Requires= starts the required unit"
    printf '[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=true\n' > /run/systemd/system/dep-required.service
    printf '[Unit]\nRequires=dep-required.service\nAfter=dep-required.service\n[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=true\n' > /run/systemd/system/dep-requirer.service
    systemctl daemon-reload
    systemctl start dep-requirer.service
    sleep 1
    systemctl is-active dep-required.service
    systemctl is-active dep-requirer.service
    systemctl stop dep-requirer.service dep-required.service
    SDEOF
  '';
}
