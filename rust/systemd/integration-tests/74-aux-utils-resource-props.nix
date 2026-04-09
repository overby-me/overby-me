{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.resource\\-props\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.resource-props.sh << 'RPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "MemoryCurrent property exists for service"
    systemctl show -P MemoryCurrent systemd-journald.service > /dev/null

    : "TasksCurrent property exists for service"
    systemctl show -P TasksCurrent systemd-journald.service > /dev/null

    : "CPUUsageNSec property exists for service"
    systemctl show -P CPUUsageNSec systemd-journald.service > /dev/null
    RPEOF
    chmod +x TEST-74-AUX-UTILS.resource-props.sh
  '';
}
