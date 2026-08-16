{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-scope\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-scope.sh << 'SCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "init.scope exists and is active"
    systemctl show init.scope -P ActiveState | grep -q "active"
    systemctl show init.scope -P Id | grep -q "init.scope"
    SCEOF
    chmod +x TEST-74-AUX-UTILS.show-scope.sh
  '';
}
