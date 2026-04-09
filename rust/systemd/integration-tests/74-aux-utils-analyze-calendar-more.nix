{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.analyze\\-calendar\\-more\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.analyze-calendar-more.sh << 'ACMEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-analyze calendar handles weekly"
    OUT="$(systemd-analyze calendar weekly 2>&1)" || true
    echo "$OUT" | grep -qi "next\|original\|normalized"

    : "systemd-analyze calendar handles monthly"
    OUT="$(systemd-analyze calendar monthly 2>&1)" || true
    echo "$OUT" | grep -qi "next\|original\|normalized"

    : "systemd-analyze calendar handles Mon..Fri expression"
    OUT="$(systemd-analyze calendar "Mon,Tue *-*-* 00:00:00" 2>&1)" || true
    echo "$OUT" | grep -qi "next\|original\|normalized"

    : "systemd-analyze calendar rejects invalid expression"
    (! systemd-analyze calendar "not-a-valid-calendar" 2>/dev/null)
    ACMEOF
    chmod +x TEST-74-AUX-UTILS.analyze-calendar-more.sh
  '';
}
