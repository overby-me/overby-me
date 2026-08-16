{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-unit\\-types\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-unit-types.sh << 'UTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemctl show works for socket units"
    # Check a known socket unit
    SSTATE="$(systemctl show -P ActiveState systemd-journald.socket)"
    [[ "$SSTATE" == "active" ]]

    : "systemctl show works for target units"
    TSTATE="$(systemctl show -P ActiveState multi-user.target)"
    [[ "$TSTATE" == "active" ]]

    : "systemctl show -P LoadState for non-existent unit"
    LSTATE="$(systemctl show -P LoadState nonexistent-unit-xyz.service)"
    [[ "$LSTATE" == "not-found" ]]

    : "systemctl show -P UnitFileState"
    UFSTATE="$(systemctl show -P UnitFileState systemd-journald.service)"
    echo "UnitFileState=$UFSTATE"
    # Should be one of: enabled, static, disabled, etc.
    [[ -n "$UFSTATE" ]]
    UTEOF
    chmod +x TEST-74-AUX-UTILS.show-unit-types.sh
  '';
}
