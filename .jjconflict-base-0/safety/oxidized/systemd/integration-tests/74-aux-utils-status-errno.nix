{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.status\\-errno\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.status-errno.sh << 'SEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "StatusErrno is 0 for successful service"
    UNIT="errno-ok-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    SE="$(systemctl show -P StatusErrno "$UNIT.service")"
    [[ "$SE" == "0" ]]
    SEEOF
    chmod +x TEST-74-AUX-UTILS.status-errno.sh
  '';
}
