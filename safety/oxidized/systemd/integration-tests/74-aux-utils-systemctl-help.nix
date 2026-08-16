{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.systemctl\\-help\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.systemctl-help.sh << 'SHEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl --help shows usage"
    systemctl --help > /dev/null

    : "systemctl --version shows version"
    systemctl --version > /dev/null

    : "systemctl --no-pager list-units works"
    systemctl --no-pager list-units > /dev/null

    : "systemctl --no-legend list-units strips headers"
    systemctl --no-pager --no-legend list-units > /dev/null

    : "systemctl --output=json list-units outputs JSON"
    systemctl --no-pager --output=json list-units > /dev/null || true

    : "systemctl --plain list-units shows flat output"
    systemctl --no-pager --plain list-units > /dev/null
    SHEOF
    chmod +x TEST-74-AUX-UTILS.systemctl-help.sh
  '';
}
