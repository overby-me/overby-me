{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.wants-after-failed\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.wants-after-failed.sh << 'WAFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    BAD="dep-badwant-$RANDOM"
    WANT="dep-softwanter-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$WANT.service" "$BAD.service" 2>/dev/null
        systemctl reset-failed "$WANT.service" "$BAD.service" 2>/dev/null
        rm -f "/run/systemd/system/$BAD.service" "/run/systemd/system/$WANT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    # The wanted dependency fails.
    cat > "/run/systemd/system/$BAD.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=false
    EOF

    # Wants= is a SOFT dependency and After= is pure ordering, so the wanter must
    # still start even though the dep failed. (Requires= would propagate the
    # failure; Wants= must not.)
    cat > "/run/systemd/system/$WANT.service" << EOF
    [Unit]
    Wants=$BAD.service
    After=$BAD.service
    [Service]
    Type=oneshot
    RemainAfterExit=yes
    ExecStart=true
    EOF
    systemctl daemon-reload

    : "Starting the wanter succeeds despite the soft dependency failing"
    systemctl start "$WANT.service"

    : "The wanter is active (a failed Wants=+After= dep must not block it)"
    systemctl is-active "$WANT.service"

    : "The failed dependency is indeed not active"
    (! systemctl is-active "$BAD.service")
    WAFEOF
  '';
}
