{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.add\\-wants\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.add-wants.sh << 'AWEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl add-wants creates .wants symlink"
    UNIT="aw-svc-$RANDOM"
    cat > "/run/systemd/system/$UNIT.service" << EOF
    [Service]
    Type=oneshot
    ExecStart=/run/current-system/sw/bin/true
    EOF
    systemctl daemon-reload
    systemctl add-wants multi-user.target "$UNIT.service"
    # The .wants symlink must now exist under the target's drop-in dir.
    test -L "/etc/systemd/system/multi-user.target.wants/$UNIT.service"
    systemctl daemon-reload
    rm -f "/run/systemd/system/$UNIT.service"
    rm -f "/etc/systemd/system/multi-user.target.wants/$UNIT.service" 2>/dev/null || true
    systemctl daemon-reload
    AWEOF
    chmod +x TEST-74-AUX-UTILS.add-wants.sh
  '';
}
