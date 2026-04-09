{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.mask\\-ops\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.mask-ops.sh << 'MKEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl mask creates /dev/null symlink"
    UNIT="mask-ops-$RANDOM"
    cat > "/run/systemd/system/$UNIT.service" << UEOF
    [Unit]
    Description=Mask test
    [Service]
    Type=oneshot
    ExecStart=true
    UEOF
    systemctl daemon-reload
    systemctl mask "$UNIT.service"
    STATE="$(systemctl is-enabled "$UNIT.service" 2>&1 || true)"
    [[ "$STATE" == "masked" || "$STATE" == *"masked"* ]]
    systemctl unmask "$UNIT.service"
    rm -f "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload
    MKEOF
    chmod +x TEST-74-AUX-UTILS.mask-ops.sh
  '';
}
