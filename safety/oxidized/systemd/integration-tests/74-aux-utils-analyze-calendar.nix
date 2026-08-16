{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.analyze\\-calendar\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.analyze-calendar.sh << 'ACEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-analyze calendar weekly"
    OUT="$(systemd-analyze calendar "weekly")"
    echo "$OUT" | grep -q "Next"

    : "systemd-analyze calendar monthly"
    OUT="$(systemd-analyze calendar "monthly")"
    echo "$OUT" | grep -q "Next"

    : "systemd-analyze calendar yearly"
    OUT="$(systemd-analyze calendar "yearly")"
    echo "$OUT" | grep -q "Next"

    : "systemd-analyze calendar with day of week"
    systemd-analyze calendar "Fri *-*-* 18:00:00" > /dev/null

    : "systemd-analyze calendar minutely"
    OUT="$(systemd-analyze calendar "minutely")"
    echo "$OUT" | grep -q "Next"

    : "systemd-analyze timespan formats"
    systemd-analyze timespan "0"
    systemd-analyze timespan "1us"
    systemd-analyze timespan "1s 500ms"
    systemd-analyze timespan "2h 30min 10s"
    systemd-analyze timespan "infinity"

    : "systemd-analyze timestamp formats"
    systemd-analyze timestamp "2025-01-01 00:00:00"
    systemd-analyze timestamp "2025-06-15 12:30:00 UTC"
    ACEOF
    chmod +x TEST-74-AUX-UTILS.analyze-calendar.sh
  '';
}
