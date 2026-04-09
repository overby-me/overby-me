{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.forking-pidfile\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.forking-pidfile.sh << 'FPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="test-forking-pidfile-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" "/run/$UNIT.pid"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    printf '[Service]\nType=forking\nPIDFile=/run/%s.pid\nExecStart=bash -c '"'"'sleep infinity & echo $! > /run/%s.pid'"'"'\n' "$UNIT" "$UNIT" > "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    sleep 1

    # Verify the service is active and PID was tracked
    systemctl is-active "$UNIT.service"
    MAIN_PID="$(systemctl show -P MainPID "$UNIT.service")"
    [[ "$MAIN_PID" -gt 0 ]]
    # Verify the PID matches what was written to the PID file
    FILE_PID="$(cat "/run/$UNIT.pid")"
    [[ "$MAIN_PID" == "$FILE_PID" ]]

    systemctl stop "$UNIT.service"
    FPEOF
  '';
}
