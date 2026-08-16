{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-multi\\-p\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-multi-p.sh << 'SMPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show multiple properties on separate calls"
    UNIT="multi-p-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    TYPE="$(systemctl show -P Type "$UNIT.service")"
    [[ "$TYPE" == "simple" ]]
    RESULT="$(systemctl show -P Result "$UNIT.service")"
    [[ "$RESULT" == "success" ]]
    SMPEOF
    chmod +x TEST-74-AUX-UTILS.show-multi-p.sh
  '';
}
