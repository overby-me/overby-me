{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.restart\\-usec\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.restart-usec.sh << 'RUEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "RestartUSec property exists"
    systemctl show -P RestartUSec systemd-journald.service > /dev/null

    : "TimeoutStartUSec property exists"
    systemctl show -P TimeoutStartUSec systemd-journald.service > /dev/null

    : "TimeoutStopUSec property exists"
    systemctl show -P TimeoutStopUSec systemd-journald.service > /dev/null
    RUEOF
    chmod +x TEST-74-AUX-UTILS.restart-usec.sh
  '';
}
