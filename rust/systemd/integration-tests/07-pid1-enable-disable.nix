{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.enable-disable\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.enable-disable.sh << 'EDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="enable-test-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl unmask "$UNIT.service" 2>/dev/null
        systemctl disable "$UNIT.service" 2>/dev/null
        rm -f "/usr/lib/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    cat > "/usr/lib/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=true
    [Install]
    WantedBy=multi-user.target
    EOF
    systemctl daemon-reload

    : "Enable creates symlink"
    (! systemctl is-enabled "$UNIT.service")
    systemctl enable "$UNIT.service"
    systemctl is-enabled "$UNIT.service"

    : "Disable removes symlink"
    systemctl disable "$UNIT.service"
    (! systemctl is-enabled "$UNIT.service")

    : "Mask creates /dev/null symlink"
    systemctl mask "$UNIT.service"
    test -L "/etc/systemd/system/$UNIT.service"
    readlink "/etc/systemd/system/$UNIT.service" | grep -q /dev/null

    : "Unmask removes the symlink"
    systemctl unmask "$UNIT.service"
    test ! -L "/etc/systemd/system/$UNIT.service"

    : "Re-enable after unmask works"
    systemctl enable "$UNIT.service"
    systemctl is-enabled "$UNIT.service"
    systemctl disable "$UNIT.service"
    EDEOF
  '';
}
