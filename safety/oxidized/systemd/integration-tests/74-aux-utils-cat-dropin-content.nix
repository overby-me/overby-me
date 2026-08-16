{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cat\\-dropin\\-content\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.cat-dropin-content.sh << 'CDCEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "Create unit with drop-in and verify cat shows both"
    UNIT="cat-drop-$RANDOM"
    cat > "/run/systemd/system/$UNIT.service" << UEOF
    [Unit]
    Description=Cat dropin test
    [Service]
    Type=oneshot
    ExecStart=true
    UEOF
    mkdir -p "/run/systemd/system/$UNIT.service.d"
    cat > "/run/systemd/system/$UNIT.service.d/override.conf" << UEOF
    [Service]
    Environment=CATTEST=yes
    UEOF
    systemctl daemon-reload
    OUT="$(systemctl cat "$UNIT.service")"
    echo "$OUT" | grep -q "Cat dropin test"
    echo "$OUT" | grep -q "CATTEST=yes"
    rm -rf "/run/systemd/system/$UNIT.service" "/run/systemd/system/$UNIT.service.d"
    systemctl daemon-reload
    CDCEOF
    chmod +x TEST-74-AUX-UTILS.cat-dropin-content.sh
  '';
}
