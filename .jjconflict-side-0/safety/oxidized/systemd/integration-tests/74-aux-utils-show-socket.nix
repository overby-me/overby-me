{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-socket\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-socket.sh << 'SSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show for systemd-journald.socket"
    systemctl show systemd-journald.socket -P ActiveState > /dev/null
    systemctl show systemd-journald.socket -P Id | grep -q "systemd-journald.socket"
    SSEOF
    chmod +x TEST-74-AUX-UTILS.show-socket.sh
  '';
}
