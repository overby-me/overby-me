{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.part-of\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.part-of.sh << 'POEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT_A="partof-a-$RANDOM"
    UNIT_B="partof-b-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT_A.service" "$UNIT_B.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT_A.service" "/run/systemd/system/$UNIT_B.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    : "PartOf= does not require the target but propagates its stop"
    cat > "/run/systemd/system/$UNIT_B.service" << EOF
    [Service]
    ExecStart=sleep infinity
    EOF
    cat > "/run/systemd/system/$UNIT_A.service" << EOF
    [Unit]
    PartOf=$UNIT_B.service
    [Service]
    ExecStart=sleep infinity
    EOF
    systemctl daemon-reload

    # Unlike BindsTo=/Requires=, PartOf= does not pull in the target: A starts
    # on its own and B stays inactive.
    systemctl start "$UNIT_A.service"
    systemctl is-active "$UNIT_A.service"
    (! systemctl is-active "$UNIT_B.service")

    # But stopping the target propagates the stop to the part unit.
    systemctl start "$UNIT_B.service"
    systemctl stop "$UNIT_B.service"
    for i in 1 2 3 4 5; do systemctl is-active "$UNIT_A.service" 2>/dev/null || break; sleep 1; done
    (! systemctl is-active "$UNIT_A.service")
    POEOF
  '';
}
