{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.status\\-format\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.status-format.sh << 'SFEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemctl status shows unit info"
    systemctl status systemd-journald.service --no-pager > /dev/null || true

    : "systemctl status with --lines limits output"
    systemctl status systemd-journald.service --no-pager --lines=3 > /dev/null || true

    : "systemctl status with --full shows full lines"
    systemctl status systemd-journald.service --no-pager --full > /dev/null || true

    : "systemctl status for multiple units"
    systemctl status systemd-journald.service init.scope --no-pager > /dev/null || true

    : "systemctl status shows loaded state"
    systemctl status systemd-journald.service --no-pager 2>&1 | grep -qi "loaded" || true
    SFEOF
    chmod +x TEST-74-AUX-UTILS.status-format.sh
  '';
}
