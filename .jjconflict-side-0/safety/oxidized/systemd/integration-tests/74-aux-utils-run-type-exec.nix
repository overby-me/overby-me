{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-type\\-exec\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-type-exec.sh << 'RTEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run --service-type=exec starts service"
    UNIT="run-type-exec-$RANDOM"
    systemd-run --unit="$UNIT" --service-type=exec sleep 300
    sleep 1
    [[ "$(systemctl show -P Type "$UNIT.service")" == "exec" ]]
    systemctl stop "$UNIT.service"
    RTEEOF
    chmod +x TEST-74-AUX-UTILS.run-type-exec.sh
  '';
}
