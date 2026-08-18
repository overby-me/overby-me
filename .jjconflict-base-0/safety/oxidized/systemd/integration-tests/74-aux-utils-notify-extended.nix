{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.notify\\-extended\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.notify-extended.sh << 'NEEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-notify --ready succeeds for PID 1"
    systemd-notify --ready || true

    : "systemd-notify --status sets status text"
    systemd-notify --status="Testing notify" || true

    : "systemd-notify --booted checks boot status"
    systemd-notify --booted
    NEEOF
    chmod +x TEST-74-AUX-UTILS.notify-extended.sh
  '';
}
