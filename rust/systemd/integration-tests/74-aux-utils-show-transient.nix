{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-transient\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-transient.sh << 'STEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "Transient service shows correct Description"
    UNIT="show-trans-$RANDOM"
    systemd-run --unit="$UNIT" --description="Show transient test" \
        --remain-after-exit true
    sleep 1
    [[ "$(systemctl show -P Description "$UNIT.service")" == "Show transient test" ]]
    [[ "$(systemctl show -P ActiveState "$UNIT.service")" == "active" ]]
    [[ "$(systemctl show -P LoadState "$UNIT.service")" == "loaded" ]]

    : "Transient service MainPID is set"
    # For remain-after-exit, the process has exited but MainPID was tracked
    systemctl show -P MainPID "$UNIT.service" > /dev/null

    : "Transient service has correct Type"
    # Default type for systemd-run is simple
    TYPE="$(systemctl show -P Type "$UNIT.service")"
    [[ "$TYPE" == "simple" || "$TYPE" == "exec" ]]
    systemctl stop "$UNIT.service" 2>/dev/null || true

    : "Oneshot transient shows Result=success after completion"
    UNIT2="show-trans2-$RANDOM"
    systemd-run --unit="$UNIT2" -p Type=oneshot -p RemainAfterExit=yes true
    sleep 1
    RESULT="$(systemctl show -P Result "$UNIT2.service")"
    [[ "$RESULT" == "success" ]]
    systemctl stop "$UNIT2.service" 2>/dev/null || true
    STEOF
    chmod +x TEST-74-AUX-UTILS.show-transient.sh
  '';
}
