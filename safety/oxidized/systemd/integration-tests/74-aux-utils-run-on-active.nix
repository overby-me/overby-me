{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-on\\-active\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-on-active.sh << 'ROAEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run --on-active creates transient timer"
    systemd-run --on-active=1s --unit="run-onactive-$RANDOM" touch /tmp/on-active-ran
    sleep 3
    [[ -f /tmp/on-active-ran ]]
    rm -f /tmp/on-active-ran

    : "systemd-run --on-boot creates timer with OnBootSec"
    UNIT="run-onboot-$RANDOM"
    systemd-run --on-boot=999h --unit="$UNIT" true
    # Just verify the timer was created and is active
    systemctl is-active "$UNIT.timer"
    systemctl stop "$UNIT.timer"
    ROAEOF
    chmod +x TEST-74-AUX-UTILS.run-on-active.sh
  '';
}
