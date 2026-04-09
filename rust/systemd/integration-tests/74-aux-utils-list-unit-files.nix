{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-unit\\-files\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-unit-files.sh << 'LUFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-unit-files shows installed units"
    systemctl list-unit-files --no-pager | grep -q ".service"

    : "systemctl list-unit-files --type=service filters by type"
    systemctl list-unit-files --no-pager --type=service | grep -q ".service"

    : "systemctl list-unit-files --state=enabled shows enabled units"
    systemctl list-unit-files --no-pager --state=enabled | grep -q "enabled" || true

    : "systemctl list-unit-files accepts a pattern"
    systemctl list-unit-files --no-pager "systemd-*" | grep -q "systemd-"
    LUFEOF
    chmod +x TEST-74-AUX-UTILS.list-unit-files.sh
  '';
}
