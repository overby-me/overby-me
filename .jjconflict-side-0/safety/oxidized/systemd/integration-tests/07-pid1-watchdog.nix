{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.watchdog\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.watchdog.sh << 'WDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="watchdog-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "WatchdogSec= kills a service that never pings (Result=watchdog)"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    ExecStart=sleep infinity
    WatchdogSec=2
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    systemctl is-active "$UNIT.service"
    # sleep never sends WATCHDOG=1, so after ~WatchdogSec the manager kills it
    # and records Result=watchdog.
    for i in $(seq 1 20); do
        R="$(systemctl show -P Result "$UNIT.service")"
        [[ "$R" == "watchdog" ]] && break
        sleep 1
    done
    [[ "$R" == "watchdog" ]]
    (! systemctl is-active "$UNIT.service")
    WDEOF
  '';
}
