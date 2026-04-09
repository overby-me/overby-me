{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-value\\-flag\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-value-flag.sh << 'SVFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show --value shows raw value"
    VAL="$(systemctl show --value -p LoadState systemd-journald.service)"
    [[ "$VAL" == "loaded" ]]

    : "systemctl show --value -p ActiveState works"
    VAL="$(systemctl show --value -p ActiveState systemd-journald.service)"
    [[ "$VAL" == "active" ]]
    SVFEOF
    chmod +x TEST-74-AUX-UTILS.show-value-flag.sh
  '';
}
