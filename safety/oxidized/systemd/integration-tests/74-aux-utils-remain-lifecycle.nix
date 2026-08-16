{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.remain\\-lifecycle\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.remain-lifecycle.sh << 'RLEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "remain-after-exit keeps unit active"
    UNIT="remain-lc-$RANDOM"
    systemd-run --unit="$UNIT" --remain-after-exit true
    sleep 1
    systemctl is-active "$UNIT.service"
    systemctl stop "$UNIT.service"
    (! systemctl is-active "$UNIT.service")
    RLEOF
    chmod +x TEST-74-AUX-UTILS.remain-lifecycle.sh
  '';
}
