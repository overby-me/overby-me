{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cgroup-limits\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.cgroup-limits.sh << 'CLEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="cglimit-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "MemoryMax= and TasksMax= are written to the service cgroup"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    ExecStart=sleep infinity
    MemoryMax=64M
    TasksMax=50
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    systemctl is-active "$UNIT.service"
    CG="$(systemctl show -P ControlGroup "$UNIT.service")"
    # MemoryMax=64M -> 64*1024*1024 bytes in memory.max.
    [[ "$(cat "/sys/fs/cgroup$CG/memory.max")" == "67108864" ]]
    # TasksMax=50 -> pids.max.
    [[ "$(cat "/sys/fs/cgroup$CG/pids.max")" == "50" ]]
    systemctl stop "$UNIT.service"
    CLEOF
  '';
}
