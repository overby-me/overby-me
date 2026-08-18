{
  name = "74-AUX-UTILS";
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.analyze\\-timespan\\.sh$";
  };
  patchScript = ''
    cat > TEST-74-AUX-UTILS.analyze-timespan.sh << 'ATEOF'
    #!/usr/bin/env bash
    set -eux
    set -o pipefail

    : "systemd-analyze timespan parses time strings"
    OUT="$(systemd-analyze timespan "5s")"
    echo "$OUT" | grep -q "5s"

    : "systemd-analyze timespan handles complex strings"
    OUT="$(systemd-analyze timespan "1h 30min")"
    echo "$OUT" | grep -q "1h 30min"

    : "systemd-analyze timespan handles microseconds"
    OUT="$(systemd-analyze timespan "500ms")"
    echo "$OUT" | grep -q "500ms"
    ATEOF
    chmod +x TEST-74-AUX-UTILS.analyze-timespan.sh
  '';
}
