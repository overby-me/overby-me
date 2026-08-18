{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cpu-quota\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.cpu-quota.sh << 'CQEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="cpuquota-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "CPUQuota= reaches the service cgroup cpu.max"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    ExecStart=sleep infinity
    CPUQuota=50%
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    systemctl is-active "$UNIT.service"
    CG="$(systemctl show -P ControlGroup "$UNIT.service")"
    # CPUQuota=50% with the default 100ms period -> "50000 100000" in cpu.max.
    [[ "$(cat "/sys/fs/cgroup$CG/cpu.max")" == "50000 100000" ]]
    systemctl stop "$UNIT.service"
    CQEOF
  '';
}
