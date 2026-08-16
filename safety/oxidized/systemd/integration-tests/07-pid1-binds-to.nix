{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.binds-to\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.binds-to.sh << 'BTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT_A="binds-a-$RANDOM"
    UNIT_B="binds-b-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT_A.service" "$UNIT_B.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT_A.service" "/run/systemd/system/$UNIT_B.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "BindsTo= stops the bound unit when its target stops"
    cat > "/run/systemd/system/$UNIT_B.service" << EOF
    [Service]
    ExecStart=sleep infinity
    EOF
    cat > "/run/systemd/system/$UNIT_A.service" << EOF
    [Unit]
    BindsTo=$UNIT_B.service
    [Service]
    ExecStart=sleep infinity
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT_B.service"
    systemctl start "$UNIT_A.service"
    systemctl is-active "$UNIT_A.service"
    systemctl is-active "$UNIT_B.service"
    # Stopping the binding target must stop the bound unit too.
    systemctl stop "$UNIT_B.service"
    for i in 1 2 3 4 5; do systemctl is-active "$UNIT_A.service" 2>/dev/null || break; sleep 1; done
    (! systemctl is-active "$UNIT_A.service")
    BTEOF
  '';
}
