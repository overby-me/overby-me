{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.run\\-working\\-dir\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.run-working-dir.sh << 'RWDEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-run --working-directory sets cwd"
    UNIT="run-cwd-$RANDOM"
    systemd-run --wait --unit="$UNIT" --working-directory=/var true
    WD="$(systemctl show -P WorkingDirectory "$UNIT.service")"
    [[ "$WD" == "/var" ]]
    RWDEOF
    chmod +x TEST-74-AUX-UTILS.run-working-dir.sh
  '';
}
