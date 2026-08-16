{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.isolate\\-target\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.isolate-target.sh << 'ITEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl get-default shows current default target"
    DEFAULT="$(systemctl get-default)"
    [[ -n "$DEFAULT" ]]

    : "systemctl set-default changes default target"
    OLD_DEFAULT="$(systemctl get-default)"
    systemctl set-default multi-user.target
    [[ "$(systemctl get-default)" == "multi-user.target" ]]
    # Restore original
    systemctl set-default "$OLD_DEFAULT"
    ITEOF
    chmod +x TEST-74-AUX-UTILS.isolate-target.sh
  '';
}
