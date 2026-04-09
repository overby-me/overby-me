{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.dep\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.dep-props.sh << 'DPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "After property is non-empty for multi-user.target"
    AFTER="$(systemctl show -P After multi-user.target)"
    [[ -n "$AFTER" ]]

    : "Wants property is non-empty for multi-user.target"
    WANTS="$(systemctl show -P Wants multi-user.target)"
    [[ -n "$WANTS" ]]
    DPEOF
    chmod +x TEST-74-AUX-UTILS.dep-props.sh
  '';
}
