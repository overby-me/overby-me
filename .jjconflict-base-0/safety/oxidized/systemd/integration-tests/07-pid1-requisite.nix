{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.requisite\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.requisite.sh << 'RQEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT_A="requisite-a-$RANDOM"
    UNIT_B="requisite-b-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT_A.service" "$UNIT_B.service" 2>/dev/null
        systemctl reset-failed "$UNIT_A.service" "$UNIT_B.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT_A.service" "/run/systemd/system/$UNIT_B.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    cat > "/run/systemd/system/$UNIT_B.service" << EOF
    [Service]
    ExecStart=sleep infinity
    EOF
    cat > "/run/systemd/system/$UNIT_A.service" << EOF
    [Unit]
    Requisite=$UNIT_B.service
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=true
    EOF
    systemctl daemon-reload

    : "Requisite= fails the start when the target is not already active"
    # B is inactive; A must fail AND must NOT pull in / start B.
    (! systemctl start "$UNIT_A.service" 2>/dev/null)
    (! systemctl is-active "$UNIT_A.service")
    (! systemctl is-active "$UNIT_B.service")

    : "Requisite= succeeds when the target is already active"
    systemctl start "$UNIT_B.service"
    systemctl is-active "$UNIT_B.service"
    systemctl start "$UNIT_A.service"
    systemctl is-active "$UNIT_A.service"
    RQEOF
  '';
}
