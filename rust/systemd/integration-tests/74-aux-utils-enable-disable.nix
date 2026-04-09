{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.enable\\-disable\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.enable-disable.sh << 'ENEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl enable creates symlink"
    UNIT="en-test-$RANDOM"
    cat > "/run/systemd/system/$UNIT.service" << UEOF
    [Unit]
    Description=Enable test
    [Service]
    Type=oneshot
    ExecStart=true
    [Install]
    WantedBy=multi-user.target
    UEOF
    systemctl daemon-reload
    systemctl enable "$UNIT.service"
    systemctl is-enabled "$UNIT.service"
    systemctl disable "$UNIT.service"
    (! systemctl is-enabled "$UNIT.service" 2>/dev/null) || true
    rm -f "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload
    ENEOF
    chmod +x TEST-74-AUX-UTILS.enable-disable.sh
  '';
}
