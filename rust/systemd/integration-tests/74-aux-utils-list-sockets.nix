{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.list\\-sockets\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.list-sockets.sh << 'LSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl list-sockets runs without error"
    systemctl list-sockets --no-pager > /dev/null

    : "systemctl list-sockets --all shows sockets"
    OUT="$(systemctl list-sockets --no-pager --all)"
    echo "$OUT" | grep -q "socket"
    LSEOF
    chmod +x TEST-74-AUX-UTILS.list-sockets.sh
  '';
}
