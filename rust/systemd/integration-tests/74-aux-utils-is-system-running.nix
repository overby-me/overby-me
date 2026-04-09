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

    : "systemctl is-system-running returns a known state"
    STATE="$(systemctl is-system-running)"
    [[ "$STATE" == "running" || "$STATE" == "degraded" || "$STATE" == "starting" ]]
    ISREOF
    chmod +x TEST-74-AUX-UTILS.is-system-running.sh
  '';
}
