{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.analyze\\-timestamp\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.analyze-timestamp.sh << 'ATSEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    # A non-empty check passed even on error (2>&1 captured the message); assert
    # the command succeeds and emits the normalized timestamp line instead.
    : "systemd-analyze timestamp parses dates"
    systemd-analyze timestamp "2024-01-01 00:00:00" | grep -q "Normalized form:"

    : "systemd-analyze timestamp parses 'now'"
    systemd-analyze timestamp now | grep -q "Normalized form:"
    ATSEOF
    chmod +x TEST-74-AUX-UTILS.analyze-timestamp.sh
  '';
}
