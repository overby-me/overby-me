{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-units\\-pattern\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-units-pattern.sh << 'LUPEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-units with glob pattern"
    OUT="$(systemctl list-units --no-pager "systemd-*" 2>/dev/null)" || true
    echo "$OUT" | grep -q "systemd-"

    : "systemctl list-units --all shows inactive too"
    systemctl list-units --no-pager --all > /dev/null

    : "systemctl list-unit-files returns output"
    OUT="$(systemctl list-unit-files --no-pager)"
    [[ -n "$OUT" ]]
    LUPEOF
    chmod +x TEST-74-AUX-UTILS.list-units-pattern.sh
  '';
}
