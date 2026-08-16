{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.nrestarts\\-prop\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.nrestarts-prop.sh << 'NRPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "NRestarts=0 for fresh service"
    UNIT="nrestart-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    NR="$(systemctl show -P NRestarts "$UNIT.service")"
    [[ "$NR" == "0" ]]
    NRPEOF
    chmod +x TEST-74-AUX-UTILS.nrestarts-prop.sh
  '';
}
