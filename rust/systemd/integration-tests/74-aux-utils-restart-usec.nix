{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.restart\\-usec\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.restart-usec.sh << 'RUEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "RestartUSec property exists"
    systemctl show -P RestartUSec systemd-journald.service > /dev/null

    : "TimeoutStartUSec property exists"
    systemctl show -P TimeoutStartUSec systemd-journald.service > /dev/null

    : "TimeoutStopUSec property exists"
    systemctl show -P TimeoutStopUSec systemd-journald.service > /dev/null

    : "RestartUSec/TimeoutStartUSec/TimeoutStopUSec reflect the configured seconds"
    UNIT="rusec-$RANDOM"
    cat > "/run/systemd/system/$UNIT.service" << EOF2
    [Service]
    ExecStart=sleep infinity
    RestartSec=5
    TimeoutStartSec=10
    TimeoutStopSec=15
    EOF2
    systemctl daemon-reload
    [[ "$(systemctl show -P RestartUSec "$UNIT.service")" == "5000000us" ]]
    [[ "$(systemctl show -P TimeoutStartUSec "$UNIT.service")" == "10000000us" ]]
    [[ "$(systemctl show -P TimeoutStopUSec "$UNIT.service")" == "15000000us" ]]
    rm -f "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload
    RUEOF
    chmod +x TEST-74-AUX-UTILS.restart-usec.sh
  '';
}
