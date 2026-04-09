{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.start\\-stop\\-lifecycle\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.start-stop-lifecycle.sh << 'SSLEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "Full start/stop lifecycle"
    UNIT="lifecycle-$RANDOM"
    cat > "/run/systemd/system/$UNIT.service" << UEOF
    [Unit]
    Description=Lifecycle test
    [Service]
    Type=exec
    ExecStart=sleep 300
    UEOF
    systemctl daemon-reload

    : "Start the service"
    systemctl start "$UNIT.service"
    sleep 1
    systemctl is-active "$UNIT.service"

    : "Stop the service"
    systemctl stop "$UNIT.service"
    (! systemctl is-active "$UNIT.service")

    rm -f "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload
    SSLEOF
    chmod +x TEST-74-AUX-UTILS.start-stop-lifecycle.sh
  '';
}
