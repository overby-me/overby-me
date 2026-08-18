{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-mount\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-mount.sh << 'SMTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show for root mount"
    systemctl show "-.mount" -P Where | grep -q "/"

    : "systemctl list-units --type=mount lists mounts"
    OUT="$(systemctl list-units --no-pager --type=mount)"
    echo "$OUT" | grep -q "\.mount"
    SMTEOF
    chmod +x TEST-74-AUX-UTILS.show-mount.sh
  '';
}
