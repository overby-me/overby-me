{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cat\\-single\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.cat-single.sh << 'CMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl cat shows unit file content"
    OUT="$(systemctl cat systemd-journald.service)"
    echo "$OUT" | grep -q "journald"

    : "systemctl cat for another unit"
    OUT="$(systemctl cat systemd-logind.service)"
    echo "$OUT" | grep -q "logind"

    : "systemctl cat with nonexistent unit fails"
    (! systemctl cat nonexistent-unit-$RANDOM.service 2>/dev/null)
    CMEOF
    chmod +x TEST-74-AUX-UTILS.cat-single.sh
  '';
}
