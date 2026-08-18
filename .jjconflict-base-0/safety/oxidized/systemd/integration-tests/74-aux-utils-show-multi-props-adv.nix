{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-multi\\-props\\-adv\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-multi-props-adv.sh << 'SMPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show multiple -P properties"
    ACTIVE="$(systemctl show -P ActiveState systemd-journald.service)"
    [[ -n "$ACTIVE" ]]
    LOAD="$(systemctl show -P LoadState systemd-journald.service)"
    [[ "$LOAD" == "loaded" ]]

    : "systemctl show -p returns key=value format"
    OUT="$(systemctl show -p LoadState systemd-journald.service)"
    echo "$OUT" | grep -q "LoadState=loaded"

    : "systemctl show -p with multiple properties"
    OUT="$(systemctl show -p LoadState -p ActiveState systemd-journald.service)"
    echo "$OUT" | grep -q "LoadState="
    echo "$OUT" | grep -q "ActiveState="
    SMPEOF
    chmod +x TEST-74-AUX-UTILS.show-multi-props-adv.sh
  '';
}
