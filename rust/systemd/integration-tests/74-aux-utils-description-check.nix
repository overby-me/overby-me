{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.description\\-check\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.description-check.sh << 'DCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "Description matches for well-known units"
    DESC="$(systemctl show -P Description multi-user.target)"
    [[ -n "$DESC" ]]

    : "Description for transient service"
    UNIT="desc-chk-$RANDOM"
    systemd-run --wait --unit="$UNIT" --description="Desc Check Test" true
    DESC="$(systemctl show -P Description "$UNIT.service")"
    [[ "$DESC" == "Desc Check Test" ]]
    DCEOF
    chmod +x TEST-74-AUX-UTILS.description-check.sh
  '';
}
