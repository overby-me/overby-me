{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.cat\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.cat.sh << 'CATEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    . "$(dirname "$0")"/util.sh

    : "systemd-cat --help shows usage"
    systemd-cat --help

    : "systemd-cat --version shows version info"
    systemd-cat --version

    : "systemd-cat runs a command and exits 0"
    systemd-cat echo "hello from cat"

    : "systemd-cat -t sets identifier without error"
    echo "test message" | systemd-cat -t "cat-ident-test"

    : "systemd-cat -p sets priority without error"
    echo "warning test" | systemd-cat -p warning

    : "systemd-cat with command and identifier"
    systemd-cat -t "cat-cmd-test" echo "command mode"
    CATEOF
    chmod +x TEST-74-AUX-UTILS.cat.sh
  '';
}
