{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-slices\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-slices.sh << 'SSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show system.slice has properties"
    systemctl show system.slice -P ActiveState | grep -q "active"

    : "systemctl list-units --type=slice shows slices"
    systemctl list-units --no-pager --type=slice > /dev/null
    SSEOF
    chmod +x TEST-74-AUX-UTILS.show-slices.sh
  '';
}
