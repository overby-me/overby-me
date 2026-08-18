{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.revert\\-unit\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.revert-unit.sh << 'RUEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl revert removes overrides"
    UNIT="revert-test-$RANDOM"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=/run/current-system/sw/bin/true
    EOF
    systemctl daemon-reload
    # Create a drop-in override
    mkdir -p "/run/systemd/system/$UNIT.service.d"
    cat > "/run/systemd/system/$UNIT.service.d/override.conf" << EOF
    [Service]
    Environment=FOO=bar
    EOF
    systemctl daemon-reload
    # The drop-in override is in effect before revert.
    systemctl show -P Environment "$UNIT.service" | grep -q "FOO=bar"
    # revert must succeed and remove the drop-in dir and the runtime unit file.
    systemctl revert "$UNIT.service"
    [[ ! -e "/run/systemd/system/$UNIT.service" ]]
    [[ ! -e "/run/systemd/system/$UNIT.service.d" ]]
    rm -rf "/run/systemd/system/$UNIT.service" "/run/systemd/system/$UNIT.service.d"
    systemctl daemon-reload
    RUEOF
    chmod +x TEST-74-AUX-UTILS.revert-unit.sh
  '';
}
