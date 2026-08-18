{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.get\\-default\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.get-default.sh << 'GDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl get-default shows multi-user.target"
    DEFAULT="$(systemctl get-default)"
    [[ "$DEFAULT" == *"multi-user.target"* || "$DEFAULT" == *"graphical.target"* ]]
    GDEOF
    chmod +x TEST-74-AUX-UTILS.get-default.sh
  '';
}
