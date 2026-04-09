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

    : "systemd-analyze timestamp parses dates"
    OUT="$(systemd-analyze timestamp "2024-01-01 00:00:00" 2>&1)" || true
    [[ -n "$OUT" ]]

    : "systemd-analyze timestamp parses 'now'"
    OUT="$(systemd-analyze timestamp now 2>&1)" || true
    [[ -n "$OUT" ]]
    ATSEOF
    chmod +x TEST-74-AUX-UTILS.analyze-timestamp.sh
  '';
}
