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

    # StartLimitBurst=3 within a wide interval so all attempts fall in one window.
    # ExecStart=false makes every start fail; a failed start attempt still counts
    # against the burst.
    printf '[Unit]\nStartLimitBurst=3\nStartLimitIntervalSec=300\n[Service]\nType=oneshot\nExecStart=false\n' > "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload

    : "Three manual starts are permitted (each fails), consuming the burst"
    for _ in 1 2 3; do
        systemctl start "$UNIT.service" 2>/dev/null || true
    done

    : "The fourth manual start within the interval is refused by StartLimitBurst="
    (! systemctl start "$UNIT.service" 2>/dev/null)

    : "Result reports start-limit-hit, not the generic exit-code"
    # This is what de-vacuums the test: (! systemctl start) alone passes merely
    # because ExecStart=false fails.  Only a genuine rate-limit refusal sets
    # Result=start-limit-hit; a plain exec failure would read exit-code.
    result="$(systemctl show -P Result "$UNIT.service")"
    test "$result" = "start-limit-hit"

    : "reset-failed drops the rate-limit history so a start is permitted again"
    systemctl reset-failed "$UNIT.service"
    systemctl start "$UNIT.service" 2>/dev/null || true
    # The post-reset start ran (and failed via ExecStart=false); it must NOT have
    # been blocked by the stale rate limit, so Result is exit-code, not
    # start-limit-hit.
    result2="$(systemctl show -P Result "$UNIT.service")"
    test "$result2" != "start-limit-hit"
    SLEOF
  '';
}
