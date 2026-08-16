{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.io-weight\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.io-weight.sh << 'IWEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="ioweight-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "IOWeight= reaches the service cgroup io.weight"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    ExecStart=sleep infinity
    IOWeight=300
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    systemctl is-active "$UNIT.service"
    CG="$(systemctl show -P ControlGroup "$UNIT.service")"
    # IOWeight maps to the default entry of io.weight ("default WEIGHT").
    [[ "$(cat "/sys/fs/cgroup$CG/io.weight")" == "default 300" ]]
    systemctl stop "$UNIT.service"
    IWEOF
  '';
}
