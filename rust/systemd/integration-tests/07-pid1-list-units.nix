{
  name = "07-PID1";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list-units\\.sh$";
  };
  patchScript = ''
    sed -i '/systemctl --no-block exit 123/d' TEST-07-PID1.sh
    cat > TEST-07-PID1.list-units.sh << 'LUEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemctl list-units shows active units"
    systemctl list-units --no-pager | grep -q "multi-user.target"

    : "systemctl list-units --type filters by type"
    systemctl list-units --no-pager --type=service | grep -q "\.service"
    systemctl list-units --no-pager --type=target | grep -q "\.target"
    systemctl list-units --no-pager --type=socket | grep -q "\.socket"

    : "systemctl list-unit-files lists installed units"
    systemctl list-unit-files --no-pager | grep -q "\.service"
    LUEOF
  '';
}
