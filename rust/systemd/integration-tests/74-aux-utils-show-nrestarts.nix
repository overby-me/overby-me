{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-nrestarts\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-nrestarts.sh << 'NREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show NRestarts for new service is 0"
    UNIT="nrestart-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    NRESTARTS="$(systemctl show -P NRestarts "$UNIT.service")"
    [[ "$NRESTARTS" == "0" ]]
    NREOF
    chmod +x TEST-74-AUX-UTILS.show-nrestarts.sh
  '';
}
