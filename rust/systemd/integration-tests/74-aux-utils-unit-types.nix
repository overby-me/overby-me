{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.unit\\-types\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.unit-types.sh << 'UTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-units shows various unit types"
    systemctl list-units --no-pager --type=service > /dev/null
    systemctl list-units --no-pager --type=socket > /dev/null
    systemctl list-units --no-pager --type=target > /dev/null
    systemctl list-units --no-pager --type=mount > /dev/null
    systemctl list-units --no-pager --type=timer > /dev/null
    systemctl list-units --no-pager --type=path > /dev/null
    UTEOF
    chmod +x TEST-74-AUX-UTILS.unit-types.sh
  '';
}
