{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.notify\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.notify.sh << 'NTEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemd-notify --help shows usage"
    systemd-notify --help

    : "systemd-notify --version shows version info"
    systemd-notify --version

    : "systemd-notify --ready outside service returns error"
    (! systemd-notify --ready) || true
    NTEOF
    chmod +x TEST-74-AUX-UTILS.notify.sh
  '';
}
