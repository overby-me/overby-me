{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-targets\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-targets.sh << 'STEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show multi-user.target has correct properties"
    systemctl show multi-user.target -P ActiveState | grep -q "active"
    systemctl show multi-user.target -P Id | grep -q "multi-user.target"

    : "systemctl list-units --type=target lists targets"
    OUT="$(systemctl list-units --no-pager --type=target)"
    echo "$OUT" | grep -q "multi-user.target"
    STEOF
    chmod +x TEST-74-AUX-UTILS.show-targets.sh
  '';
}
