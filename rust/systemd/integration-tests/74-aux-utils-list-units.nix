{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-units\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-units.sh << 'LUEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemctl list-units shows loaded units"
    systemctl list-units --no-pager > /dev/null

    : "systemctl list-units --type=service shows output"
    systemctl list-units --type=service --no-pager > /dev/null

    : "systemctl list-unit-files shows unit file states"
    systemctl list-unit-files --no-pager > /dev/null

    : "systemctl list-unit-files --type=timer shows timer files"
    systemctl list-unit-files --type=timer --no-pager > /dev/null

    : "systemctl list-timers shows active timers"
    systemctl list-timers --no-pager

    : "systemctl list-sockets shows active sockets"
    systemctl list-sockets --no-pager
    LUEOF
    chmod +x TEST-74-AUX-UTILS.list-units.sh
  '';
}
