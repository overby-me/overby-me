{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.systemctl\\-misc\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.systemctl-misc.sh << 'SMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl is-system-running returns running or degraded"
    STATE=$(systemctl is-system-running || true)
    [[ "$STATE" == "running" || "$STATE" == "degraded" ]]

    : "systemctl daemon-reload succeeds"
    systemctl daemon-reload

    : "systemctl list-machines lists the local machine"
    systemctl list-machines --no-pager | grep -q "\.host"

    : "systemctl show --property=Version"
    systemctl show --property=Version | grep -q "Version="
    SMEOF
    chmod +x TEST-74-AUX-UTILS.systemctl-misc.sh
  '';
}
