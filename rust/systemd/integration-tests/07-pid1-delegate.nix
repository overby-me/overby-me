{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.delegate\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.delegate.sh << 'DGEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="delegate-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "Delegate=yes is reflected in the Delegate property"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    ExecStart=sleep infinity
    Delegate=yes
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT.service"
    [[ "$(systemctl show -P Delegate "$UNIT.service")" == "yes" ]]
    systemctl stop "$UNIT.service"

    : "A service without Delegate= reports no"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    ExecStart=sleep infinity
    EOF
    systemctl daemon-reload
    [[ "$(systemctl show -P Delegate "$UNIT.service")" == "no" ]]
    DGEOF
  '';
}
