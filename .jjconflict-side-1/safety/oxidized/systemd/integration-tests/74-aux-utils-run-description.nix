{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-description\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-description.sh << 'RDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run --description sets unit description"
    UNIT="run-desc-$RANDOM"
    systemd-run --unit="$UNIT" --description="My test description" --remain-after-exit true
    sleep 1
    DESC="$(systemctl show -P Description "$UNIT.service")"
    [[ "$DESC" == "My test description" ]]
    systemctl stop "$UNIT.service"
    RDEOF
    chmod +x TEST-74-AUX-UTILS.run-description.sh
  '';
}
