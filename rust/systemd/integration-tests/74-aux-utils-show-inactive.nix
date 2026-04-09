{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-inactive\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-inactive.sh << 'SIEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show for non-existent unit returns not-found"
    LS="$(systemctl show -P LoadState nonexistent-unit-$RANDOM.service)"
    [[ "$LS" == "not-found" ]]

    : "systemctl is-active returns inactive for non-running"
    (! systemctl is-active nonexistent-$RANDOM.service)

    : "systemctl is-failed returns true for non-existent"
    (! systemctl is-failed nonexistent-$RANDOM.service) || true

    : "systemctl show works for target units"
    [[ "$(systemctl show -P ActiveState multi-user.target)" == "active" ]]
    [[ "$(systemctl show -P LoadState multi-user.target)" == "loaded" ]]
    SIEOF
    chmod +x TEST-74-AUX-UTILS.show-inactive.sh
  '';
}
