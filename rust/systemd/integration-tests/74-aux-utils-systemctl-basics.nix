{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.systemctl\\-basics\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.systemctl-basics.sh << 'SBEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemctl --version prints version info"
    systemctl --version > /dev/null

    : "systemctl --help shows help"
    systemctl --help > /dev/null

    : "systemctl list-unit-files shows files"
    systemctl list-unit-files --no-pager > /dev/null

    : "systemctl list-units --state=active shows active units"
    systemctl list-units --no-pager --state=active > /dev/null

    : "systemctl list-units --state=inactive shows inactive units"
    systemctl list-units --no-pager --state=inactive > /dev/null

    : "systemctl show-environment prints environment"
    systemctl show-environment > /dev/null

    : "systemctl log-level returns current level"
    LEVEL="$(systemctl log-level)"
    echo "Log level: $LEVEL"
    [[ -n "$LEVEL" ]]
    SBEOF
    chmod +x TEST-74-AUX-UTILS.systemctl-basics.sh
  '';
}
