{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cgroup-weights\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.cgroup-weights.sh << 'CWEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="cgweight-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "CPUWeight= and MemoryHigh= reach the service cgroup"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    ExecStart=sleep infinity
    CPUWeight=200
    MemoryHigh=32M
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    systemctl is-active "$UNIT.service"
    CG="$(systemctl show -P ControlGroup "$UNIT.service")"
    # CPUWeight maps directly to cgroup v2 cpu.weight.
    [[ "$(cat "/sys/fs/cgroup$CG/cpu.weight")" == "200" ]]
    # MemoryHigh=32M -> 32*1024*1024 bytes in memory.high.
    [[ "$(cat "/sys/fs/cgroup$CG/memory.high")" == "33554432" ]]
    systemctl stop "$UNIT.service"
    CWEOF
  '';
}
