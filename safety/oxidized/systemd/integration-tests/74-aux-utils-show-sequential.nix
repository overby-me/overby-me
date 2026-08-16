{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.show\\-sequential\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.show-sequential.sh << 'SQEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl show for journald service"
    systemctl show systemd-journald.service -P ActiveState | grep -q "active"

    : "systemctl show for logind service"
    systemctl show systemd-logind.service -P Id | grep -q "logind"

    : "systemctl show for resolved service"
    systemctl show systemd-resolved.service -P Id | grep -q "resolved"
    SQEOF
    chmod +x TEST-74-AUX-UTILS.show-sequential.sh
  '';
}
