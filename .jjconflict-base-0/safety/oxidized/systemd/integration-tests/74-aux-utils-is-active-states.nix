{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.is\\-active\\-states\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.is-active-states.sh << 'IAEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl is-active returns active for running service"
    systemctl is-active multi-user.target

    : "systemctl is-active returns inactive for stopped service"
    UNIT="isactive-$RANDOM"
    systemd-run --wait --unit="$UNIT" true
    (! systemctl is-active "$UNIT.service")

    : "systemctl is-active for nonexistent unit returns inactive"
    (! systemctl is-active nonexistent-unit-$RANDOM.service)
    IAEOF
    chmod +x TEST-74-AUX-UTILS.is-active-states.sh
  '';
}
