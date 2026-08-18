{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.daemon\\-reload\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.daemon-reload.sh << 'DREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "daemon-reload succeeds"
    systemctl daemon-reload

    : "After reload, new unit files are picked up"
    UNIT="dr-test-$RANDOM"
    cat > "/run/systemd/system/$UNIT.service" << UEOF
    [Service]
    Type=oneshot
    ExecStart=true
    UEOF
    systemctl daemon-reload
    systemctl show -P LoadState "$UNIT.service" | grep -q "loaded"
    rm -f "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload
    DREOF
    chmod +x TEST-74-AUX-UTILS.daemon-reload.sh
  '';
}
