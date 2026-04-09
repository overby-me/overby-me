{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-uf\\-pattern\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-uf-pattern.sh << 'LUFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-unit-files with pattern filter"
    OUT="$(systemctl list-unit-files --no-pager "systemd-journald*")"
    echo "$OUT" | grep -q "journald"

    : "systemctl list-unit-files --no-legend shows compact"
    systemctl list-unit-files --no-pager --no-legend > /dev/null
    LUFEOF
    chmod +x TEST-74-AUX-UTILS.list-uf-pattern.sh
  '';
}
