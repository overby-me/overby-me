{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-path\\-unit\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-path-unit.sh << 'SPUEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "Can create and load path unit"
    UNIT="path-show-$RANDOM"
    cat > "/run/systemd/system/$UNIT.path" << UEOF
    [Path]
    PathExists=/tmp
    UEOF
    cat > "/run/systemd/system/$UNIT.service" << UEOF
    [Service]
    Type=oneshot
    ExecStart=true
    UEOF
    systemctl daemon-reload
    systemctl show "$UNIT.path" -P Id | grep -q "$UNIT.path"
    rm -f "/run/systemd/system/$UNIT.path" "/run/systemd/system/$UNIT.service"
    systemctl daemon-reload
    SPUEOF
    chmod +x TEST-74-AUX-UTILS.show-path-unit.sh
  '';
}
