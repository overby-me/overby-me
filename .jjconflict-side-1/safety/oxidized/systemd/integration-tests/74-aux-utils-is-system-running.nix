{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.is\\-system\\-running\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.is-system-running.sh << 'ISREOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl is-system-running reaches running (no spurious degraded/failed units)"
    timeout 60 bash -c 'until [[ "$(systemctl is-system-running)" == "running" ]]; do sleep 1; done'
    ISREOF
    chmod +x TEST-74-AUX-UTILS.is-system-running.sh
  '';
}
