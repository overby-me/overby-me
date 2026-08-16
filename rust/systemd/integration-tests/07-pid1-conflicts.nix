{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.conflicts\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.conflicts.sh << 'CFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT_A="conflict-a-$RANDOM"
    UNIT_B="conflict-b-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT_A.service" "$UNIT_B.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT_A.service" "/run/systemd/system/$UNIT_B.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "Conflicts= stops the running conflictor when the other unit starts"
    cat > "/run/systemd/system/$UNIT_B.service" << EOF
    [Service]
    ExecStart=sleep infinity
    EOF
    cat > "/run/systemd/system/$UNIT_A.service" << EOF
    [Unit]
    Conflicts=$UNIT_B.service
    [Service]
    ExecStart=sleep infinity
    EOF
    systemctl daemon-reload
    systemctl start "$UNIT_B.service"
    systemctl is-active "$UNIT_B.service"
    # Starting A conflicts with B, so B must be stopped.
    systemctl start "$UNIT_A.service"
    systemctl is-active "$UNIT_A.service"
    (! systemctl is-active "$UNIT_B.service")
    systemctl stop "$UNIT_A.service"
    CFEOF
  '';
}
