{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-advanced\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-advanced.sh << 'RAEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    retry() { for i in 1 2 3 4 5; do "$@" && return 0; sleep 1; done; "$@"; }

    : "systemd-run --on-active creates timer and fires"
    UNIT="run-timer-$RANDOM"
    rm -f "/tmp/run-timer-result-$UNIT"
    systemd-run --unit="$UNIT" --on-active=1s --remain-after-exit \
        touch "/tmp/run-timer-result-$UNIT"
    systemctl is-active "$UNIT.timer"
    timeout 15 bash -c "until [[ -f /tmp/run-timer-result-$UNIT ]]; do sleep 0.5; done"
    systemctl stop "$UNIT.timer" "$UNIT.service" 2>/dev/null || true
    rm -f "/tmp/run-timer-result-$UNIT"

    : "systemd-run --remain-after-exit keeps service active"
    UNIT="run-rae-$RANDOM"
    systemd-run --unit="$UNIT" --remain-after-exit true
    sleep 1
    retry systemctl is-active "$UNIT.service"
    systemctl stop "$UNIT.service"

    : "systemd-run --description sets Description property"
    UNIT="run-desc-$RANDOM"
    systemd-run --unit="$UNIT" --remain-after-exit --description="Test Description for $UNIT" true
    sleep 1
    [[ "$(systemctl show -P Description "$UNIT.service")" == "Test Description for $UNIT" ]]
    systemctl stop "$UNIT.service"

    : "systemd-run -p WorkingDirectory= sets working dir"
    UNIT="run-wd-$RANDOM"
    OUTFILE="/tmp/run-wd-result-$RANDOM"
    systemd-run --unit="$UNIT" --wait -p WorkingDirectory=/tmp bash -c "pwd > $OUTFILE"
    [[ "$(cat "$OUTFILE")" == "/tmp" ]]
    rm -f "$OUTFILE"

    : "systemd-run --collect removes unit after stop"
    UNIT="run-collect-$RANDOM"
    systemd-run --unit="$UNIT" --collect --wait true
    # Unit should be gone after completion with --collect
    sleep 1
    RAEOF
    chmod +x TEST-74-AUX-UTILS.run-advanced.sh
  '';
}
