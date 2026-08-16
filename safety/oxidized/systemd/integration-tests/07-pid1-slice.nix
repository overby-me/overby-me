{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.slice\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.slice.sh << 'SLEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="sliced-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service" /run/systemd/system/cgtest.slice
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "Slice= places the service under the given slice cgroup"
    cat > /run/systemd/system/cgtest.slice << EOF
    [Unit]
    Description=cgroup placement test slice
    EOF
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    ExecStart=sleep infinity
    Slice=cgtest.slice
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    systemctl is-active "$UNIT.service"
    CG="$(systemctl show -P ControlGroup "$UNIT.service")"
    # The service's control group must sit under cgtest.slice, not system.slice.
    [[ "$CG" == *"/cgtest.slice/$UNIT.service" ]]
    [[ -d "/sys/fs/cgroup$CG" ]]
    systemctl stop "$UNIT.service"
    SLEOF
  '';
}
