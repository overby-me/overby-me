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

    : "systemctl is-system-running reaches running (no spurious degraded/failed units)"
    timeout 60 bash -c 'until [[ "$(systemctl is-system-running)" == "running" ]]; do sleep 1; done'

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
