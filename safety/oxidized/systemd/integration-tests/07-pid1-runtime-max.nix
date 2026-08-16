{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.runtime-max\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.runtime-max.sh << 'RMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="runtime-max-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "RuntimeMaxSec= kills a long-running service after the limit (Result=timeout)"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    ExecStart=sleep infinity
    RuntimeMaxSec=2
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    systemctl is-active "$UNIT.service"
    # After ~RuntimeMaxSec the service is killed and its Result becomes timeout.
    for i in $(seq 1 20); do
        R="$(systemctl show -P Result "$UNIT.service")"
        [[ "$R" == "timeout" ]] && break
        sleep 1
    done
    [[ "$R" == "timeout" ]]
    (! systemctl is-active "$UNIT.service")
    RMEOF
  '';
}
