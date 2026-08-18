{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.env\\-manager\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.env-manager.sh << 'EMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show-environment lists manager env"
    systemctl show-environment > /dev/null

    : "systemctl set-environment sets a variable"
    systemctl set-environment TESTVAR123=hello
    OUT="$(systemctl show-environment)"
    echo "$OUT" | grep -q "TESTVAR123=hello"

    : "systemctl unset-environment removes variable"
    systemctl unset-environment TESTVAR123
    OUT="$(systemctl show-environment)"
    (! echo "$OUT" | grep -q "TESTVAR123")
    EMEOF
    chmod +x TEST-74-AUX-UTILS.env-manager.sh
  '';
}
