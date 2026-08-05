{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.resource\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.resource-props.sh << 'RPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "MemoryCurrent property exists for service"
    systemctl show -P MemoryCurrent systemd-journald.service > /dev/null

    : "TasksCurrent property exists for service"
    systemctl show -P TasksCurrent systemd-journald.service > /dev/null

    : "CPUUsageNSec property exists for service"
    systemctl show -P CPUUsageNSec systemd-journald.service > /dev/null

    : "TasksCurrent and MemoryCurrent report live cgroup counters"
    UNIT="rescur-$RANDOM"
    cat > "/run/systemd/system/$UNIT.service" << EOF2
    [Service]
    ExecStart=sleep infinity
    MemoryMax=64M
    TasksMax=50
    CPUWeight=100
    EOF2
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    # With the pids/memory/cpu controllers enabled, a running sleep has >=1 task,
    # non-zero current memory, and a numeric CPU usage counter.
    [[ "$(systemctl show -P TasksCurrent "$UNIT.service")" -ge 1 ]]
    [[ "$(systemctl show -P MemoryCurrent "$UNIT.service")" -gt 0 ]]
    [[ "$(systemctl show -P CPUUsageNSec "$UNIT.service")" =~ ^[0-9]+$ ]]
    systemctl stop "$UNIT.service"
    rm -f "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload
    RPEOF
    chmod +x TEST-74-AUX-UTILS.resource-props.sh
  '';
}
