{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.names\\-prop\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.names-prop.sh << 'NMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "Names property contains the unit name"
    NAMES="$(systemctl show -P Names systemd-journald.service)"
    echo "$NAMES" | grep -q "systemd-journald.service"
    NMEOF
    chmod +x TEST-74-AUX-UTILS.names-prop.sh
  '';
}
