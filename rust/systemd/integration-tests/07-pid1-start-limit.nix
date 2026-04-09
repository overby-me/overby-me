{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.start-limit\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.start-limit.sh << 'SLEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    UNIT="test-start-limit-$RANDOM"

    at_exit() {
        set +e
        systemctl stop "$UNIT.service" 2>/dev/null
        systemctl reset-failed "$UNIT.service" 2>/dev/null
        rm -f "/run/systemd/system/$UNIT.service"
        systemctl daemon-reload
    }
    trap at_exit EXIT

    printf '[Unit]\nStartLimitBurst=3\nStartLimitIntervalSec=30\n[Service]\nType=oneshot\nExecStart=false\n' > "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload

    # First 3 starts should be allowed (they fail, but they start)
    for i in 1 2 3; do
        systemctl start "$UNIT.service" || true
    done

    # After 3 failures within the interval, the 4th start should be refused
    (! systemctl start "$UNIT.service" 2>/dev/null)
    SLEOF
  '';
}
