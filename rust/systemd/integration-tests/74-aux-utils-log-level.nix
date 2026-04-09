{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.log\\-level\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.log-level.sh << 'LLEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl log-level shows current level"
    LEVEL="$(systemctl log-level)"
    [[ -n "$LEVEL" ]]

    : "systemctl log-level can set and restore"
    OLD_LEVEL="$(systemctl log-level)"
    systemctl log-level info
    [[ "$(systemctl log-level)" == "info" ]]
    systemctl log-level "$OLD_LEVEL"
    LLEOF
    chmod +x TEST-74-AUX-UTILS.log-level.sh
  '';
}
